param(
    [Parameter(Mandatory = $true)]
    [string]$OutputZip,
    [string]$ModelDir = ""
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$PythonCommand = (Get-Command python.exe -ErrorAction Stop).Source
$PythonBase = (& $PythonCommand -c "import sys; print(sys.base_prefix)").Trim()
$FfmpegUrl = "https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-07-10-13-44/ffmpeg-n7.1.5-1-g7d0e842004-win64-gpl-shared-7.1.zip"
$FfmpegSha256 = "19e83b78bee19a0ad1b46ad154413d05491dfc1c51d05fe0aa5acfd2b2194890"
$TargetTriple = "x86_64-pc-windows-msvc"

function Download-VerifiedArchive {
    param([string]$Url, [string]$Destination, [string]$ExpectedSha256)
    for ($attempt = 1; $attempt -le 5; $attempt++) {
        try {
            Invoke-WebRequest -Uri $Url -OutFile $Destination -UseBasicParsing
            $actual = (Get-FileHash -Algorithm SHA256 -Path $Destination).Hash.ToLowerInvariant()
            if ($actual -ne $ExpectedSha256.ToLowerInvariant()) {
                throw "checksum mismatch for $Url (expected $ExpectedSha256, got $actual)"
            }
            return
        }
        catch {
            Remove-Item -Force -ErrorAction SilentlyContinue $Destination
            if ($attempt -eq 5) { throw }
            Start-Sleep -Seconds (5 * $attempt)
        }
    }
}

$stage = Join-Path ([System.IO.Path]::GetTempPath()) ("sbobino-pyannote-" + [guid]::NewGuid())
$runtimeRoot = Join-Path $stage "python"
$sitePackages = Join-Path $runtimeRoot "Lib\site-packages"

try {
    New-Item -ItemType Directory -Force -Path $runtimeRoot | Out-Null
    foreach ($name in @("python.exe", "pythonw.exe", "python3.dll", "python311.dll")) {
        $source = Join-Path $PythonBase $name
        if (Test-Path $source) { Copy-Item $source $runtimeRoot -Force }
    }
    Get-ChildItem -Path $PythonBase -File -Filter "vcruntime*.dll" |
        ForEach-Object { Copy-Item $_.FullName $runtimeRoot -Force }
    Copy-Item (Join-Path $PythonBase "DLLs") $runtimeRoot -Recurse -Force
    Copy-Item (Join-Path $PythonBase "Lib") $runtimeRoot -Recurse -Force
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $sitePackages
    New-Item -ItemType Directory -Force -Path $sitePackages | Out-Null

    & $PythonCommand -m pip install --disable-pip-version-check `
        --target $sitePackages `
        --index-url https://download.pytorch.org/whl/cpu `
        "torch==2.9.1" "torchaudio==2.9.1" "torchcodec==0.8.1"
    if ($LASTEXITCODE -ne 0) { throw "failed to install the native PyTorch CPU stack" }

    $constraints = Join-Path $stage "constraints.txt"
    @("torch==2.9.1", "torchaudio==2.9.1", "torchcodec==0.8.1") |
        Set-Content -Encoding ASCII $constraints
    $previousPythonPath = $env:PYTHONPATH
    try {
        $env:PYTHONPATH = $sitePackages
        & $PythonCommand -m pip install --disable-pip-version-check `
            --target $sitePackages `
            --upgrade-strategy only-if-needed `
            --constraint $constraints `
            "pyannote.audio==4.0.4"
        if ($LASTEXITCODE -ne 0) { throw "failed to install pyannote.audio" }
    }
    finally {
        $env:PYTHONPATH = $previousPythonPath
    }

    $ffmpegArchive = Join-Path $stage "ffmpeg.zip"
    $ffmpegExtract = Join-Path $stage "ffmpeg"
    Download-VerifiedArchive $FfmpegUrl $ffmpegArchive $FfmpegSha256
    Expand-Archive -Path $ffmpegArchive -DestinationPath $ffmpegExtract
    Get-ChildItem -Path $ffmpegExtract -Recurse -File -Filter "*.dll" |
        ForEach-Object { Copy-Item $_.FullName (Join-Path $runtimeRoot "DLLs\$($_.Name)") -Force }

    Get-ChildItem -Path $runtimeRoot -Directory -Recurse -Filter "__pycache__" |
        Remove-Item -Recurse -Force -ErrorAction SilentlyContinue
    Get-ChildItem -Path $runtimeRoot -File -Recurse -Include "*.pyc", "*.pyo" |
        Remove-Item -Force -ErrorAction SilentlyContinue

    $runtimePython = Join-Path $runtimeRoot "python.exe"
    $previousPath = $env:PATH
    $previousHome = $env:PYTHONHOME
    $previousRuntimePythonPath = $env:PYTHONPATH
    try {
        $env:PATH = "$runtimeRoot;$runtimeRoot\DLLs;$env:SystemRoot\System32"
        $env:PYTHONHOME = $runtimeRoot
        $env:PYTHONPATH = "$runtimeRoot\Lib;$sitePackages"
        & $runtimePython -c "import os, pathlib, sys; _dll=os.add_dll_directory(str(pathlib.Path(sys.executable).parent/'DLLs')); import numpy, torch, torchaudio, torchcodec; from pyannote.audio import Pipeline; assert torch.__version__.startswith('2.9.1'); assert torchaudio.__version__.startswith('2.9.1'); print('pyannote-windows-runtime-ok')"
        if ($LASTEXITCODE -ne 0) { throw "isolated Windows pyannote runtime validation failed" }
        if ($ModelDir) {
            & $runtimePython -c "import os, pathlib, sys; _dll=os.add_dll_directory(str(pathlib.Path(sys.executable).parent/'DLLs')); from pyannote.audio import Pipeline; Pipeline.from_pretrained(r'$ModelDir'); print('pyannote-windows-model-ok')"
            if ($LASTEXITCODE -ne 0) { throw "offline pyannote model validation failed" }
        }
    }
    finally {
        $env:PATH = $previousPath
        $env:PYTHONHOME = $previousHome
        $env:PYTHONPATH = $previousRuntimePythonPath
    }

    @{
        app_target = $TargetTriple
        python_version = (& $runtimePython -c "import sys; print('.'.join(map(str, sys.version_info[:3])))").Trim()
        torch_version = "2.9.1"
        torchaudio_version = "2.9.1"
        torchcodec_version = "0.8.1"
        pyannote_audio_version = "4.0.4"
    } | ConvertTo-Json | Set-Content -Encoding UTF8 (Join-Path $runtimeRoot "sbobino-runtime.json")

    $outputParent = Split-Path -Parent $OutputZip
    if ($outputParent) { New-Item -ItemType Directory -Force -Path $outputParent | Out-Null }
    Remove-Item -Force -ErrorAction SilentlyContinue $OutputZip
    Compress-Archive -Path $runtimeRoot -DestinationPath $OutputZip -CompressionLevel Optimal
    Write-Host "Created native Windows Pyannote runtime: $OutputZip"
}
finally {
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $stage
}
