param(
    [Parameter(Mandatory = $true)]
    [string]$Version,
    [Parameter(Mandatory = $true)]
    [string]$StagingDir
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

function Assert-X64Pe {
    param([string]$Path)
    $stream = [System.IO.File]::OpenRead($Path)
    try {
        $reader = [System.IO.BinaryReader]::new($stream)
        if ($reader.ReadUInt16() -ne 0x5A4D) { throw "$Path is not a PE executable" }
        $stream.Position = 0x3C
        $peOffset = $reader.ReadInt32()
        $stream.Position = $peOffset
        if ($reader.ReadUInt32() -ne 0x00004550) { throw "$Path has no PE signature" }
        $machine = $reader.ReadUInt16()
        if ($machine -ne 0x8664) {
            throw "$Path is not x86_64 PE (machine=0x$($machine.ToString('X4')))"
        }
    }
    finally {
        $stream.Dispose()
    }
}

$required = @(
    "Sbobino_${Version}_windows_x86_64-setup.exe",
    "Sbobino_${Version}_windows_x86_64.nsis.zip",
    "Sbobino_${Version}_windows_x86_64.nsis.zip.sig",
    "speech-runtime-windows-x86_64.zip",
    "pyannote-runtime-windows-x86_64.zip"
)
foreach ($name in $required) {
    $path = Join-Path $StagingDir $name
    if (-not (Test-Path -PathType Leaf $path)) { throw "missing Windows release asset: $path" }
    if ((Get-Item $path).Length -le 0) { throw "empty Windows release asset: $path" }
}

$stage = Join-Path ([System.IO.Path]::GetTempPath()) ("sbobino-windows-readiness-" + [guid]::NewGuid())
try {
    $runtimeStage = Join-Path $stage "speech"
    $pyannoteStage = Join-Path $stage "pyannote"
    Expand-Archive (Join-Path $StagingDir "speech-runtime-windows-x86_64.zip") $runtimeStage
    Expand-Archive (Join-Path $StagingDir "pyannote-runtime-windows-x86_64.zip") $pyannoteStage

    $runtimeBin = Join-Path $runtimeStage "runtime\bin"
    $runtimeLib = Join-Path $runtimeStage "runtime\lib"
    foreach ($binary in @("ffmpeg.exe", "whisper-cli.exe", "whisper-stream.exe", "parakeet-cli.exe")) {
        $path = Join-Path $runtimeBin $binary
        if (-not (Test-Path -PathType Leaf $path)) { throw "runtime is missing $binary" }
        Assert-X64Pe $path
    }
    $runtimeDlls = @(Get-ChildItem -Path $runtimeLib -File -Filter "*.dll")
    if ($runtimeDlls.Count -eq 0) { throw "Windows speech runtime contains no DLLs" }
    $runtimeDlls | ForEach-Object { Assert-X64Pe $_.FullName }

    $previousPath = $env:PATH
    try {
        $env:PATH = "$runtimeBin;$runtimeLib;$env:SystemRoot\System32"
        & (Join-Path $runtimeBin "ffmpeg.exe") -version | Out-Null
        & (Join-Path $runtimeBin "whisper-cli.exe") --help 2>&1 | Out-Null
        & (Join-Path $runtimeBin "whisper-stream.exe") --help 2>&1 | Out-Null
        $parakeetOutput = (& (Join-Path $runtimeBin "parakeet-cli.exe") --help 2>&1 | Out-String)
        if ($LASTEXITCODE -ne 0 -and $parakeetOutput -notmatch "parakeet-cli|transcribe|Usage") {
            throw "parakeet runtime probe failed: $parakeetOutput"
        }
        $global:LASTEXITCODE = 0
    }
    finally {
        $env:PATH = $previousPath
    }

    $pythonRoot = Join-Path $pyannoteStage "python"
    $python = Join-Path $pythonRoot "python.exe"
    if (-not (Test-Path -PathType Leaf $python)) { throw "Pyannote runtime is missing python.exe" }
    Assert-X64Pe $python
    foreach ($relative in @("Lib\encodings", "Lib\collections", "Lib\site-packages", "DLLs")) {
        if (-not (Test-Path -PathType Container (Join-Path $pythonRoot $relative))) {
            throw "Pyannote runtime is missing $relative"
        }
    }

    $previousHome = $env:PYTHONHOME
    $previousPythonPath = $env:PYTHONPATH
    $previousPath = $env:PATH
    try {
        $env:PYTHONHOME = $pythonRoot
        $env:PYTHONPATH = "$pythonRoot\Lib;$pythonRoot\Lib\site-packages"
        $env:PATH = "$pythonRoot;$pythonRoot\DLLs;$runtimeBin;$runtimeLib;$env:SystemRoot\System32"
        & $python -c "import numpy, torch, torchaudio, torchcodec; from pyannote.audio import Pipeline; print('windows-pyannote-readiness-ok')"
        if ($LASTEXITCODE -ne 0) { throw "isolated Pyannote import probe failed" }
    }
    finally {
        $env:PYTHONHOME = $previousHome
        $env:PYTHONPATH = $previousPythonPath
        $env:PATH = $previousPath
    }

    $signature = (Get-Content -Raw (Join-Path $StagingDir "Sbobino_${Version}_windows_x86_64.nsis.zip.sig")).Trim()
    if (-not $signature) { throw "Windows updater signature is empty" }

    $hashes = @{}
    foreach ($name in $required) {
        $hashes[$name] = (Get-FileHash -Algorithm SHA256 (Join-Path $StagingDir $name)).Hash.ToLowerInvariant()
    }
    $proof = [ordered]@{
        gate = "windows_release_readiness.ps1"
        status = "passed"
        version = $Version
        target = "x86_64-pc-windows-msvc"
        platform = "windows-x86_64"
        sha256 = $hashes
    }
    $proof | ConvertTo-Json -Depth 5 | Set-Content -Encoding UTF8 (Join-Path $StagingDir "windows-release-readiness-proof.json")
    Write-Host "Windows release readiness passed for Sbobino $Version"
}
finally {
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $stage
}
$global:LASTEXITCODE = 0
