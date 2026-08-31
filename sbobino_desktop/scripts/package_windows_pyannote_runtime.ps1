param(
    [Parameter(Mandatory = $true)]
    [string]$OutputZip,
    [string]$ModelDir = "",
    [string]$FfmpegArchivePath = ""
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$PythonCommand = (Get-Command python.exe -ErrorAction Stop).Source
$PythonBase = (& $PythonCommand -c "import sys; print(sys.base_prefix)").Trim()
# TorchCodec requires a shared FFmpeg build on Windows. The archive is pinned
# by SHA-256; callers may provide a staged speech runtime or override the URL,
# but no unverified bytes are accepted.
$FfmpegUrl = if ($env:SBOBINO_FFMPEG_RUNTIME_URL) {
    $env:SBOBINO_FFMPEG_RUNTIME_URL
} else {
    "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-n8.1-latest-win64-gpl-shared-8.1.zip"
}
$FfmpegSha256 = if ($env:SBOBINO_FFMPEG_RUNTIME_SHA256) {
    $env:SBOBINO_FFMPEG_RUNTIME_SHA256
} else {
    "ca56a31e81af11c8e4c77249039d3a61833ec37bfbc5308da91ba1b00b43a00f"
}
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

    $ffmpegArchive = $FfmpegArchivePath
    if (-not $ffmpegArchive) {
        $ffmpegArchive = Join-Path $stage "ffmpeg.zip"
        Download-VerifiedArchive $FfmpegUrl $ffmpegArchive $FfmpegSha256
    }
    elseif (-not (Test-Path -PathType Leaf $ffmpegArchive)) {
        throw "Windows speech runtime archive was not found at '$ffmpegArchive'"
    }
    $ffmpegExtract = Join-Path $stage "ffmpeg"
    Expand-Archive -Path $ffmpegArchive -DestinationPath $ffmpegExtract
    $ffmpegExecutables = @(Get-ChildItem -Path $ffmpegExtract -Recurse -File -Filter "ffmpeg.exe")
    if ($ffmpegExecutables.Count -ne 1) {
        throw "Windows speech runtime archive must contain exactly one ffmpeg.exe"
    }
    $ffmpegDlls = @(Get-ChildItem -Path $ffmpegExtract -Recurse -File -Filter "*.dll")
    if ($ffmpegDlls.Count -eq 0) {
        throw "Windows speech runtime archive contains no FFmpeg DLLs"
    }
    $ffmpegDlls |
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
