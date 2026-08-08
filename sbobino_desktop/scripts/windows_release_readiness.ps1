param(
    [Parameter(Mandatory = $true)]
    [string]$Version,
    [Parameter(Mandatory = $true)]
    [string]$StagingDir,
    [Parameter(Mandatory = $true)]
    [string]$ParakeetModelPath
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

function Write-MinimalPcm16Wav {
    param([string]$Path)

    $sampleRate = 16000
    $sampleCount = $sampleRate
    $dataBytes = $sampleCount * 2
    $writer = [System.IO.BinaryWriter]::new([System.IO.File]::Create($Path))
    try {
        $writer.Write([System.Text.Encoding]::ASCII.GetBytes("RIFF"))
        $writer.Write([int] (36 + $dataBytes))
        $writer.Write([System.Text.Encoding]::ASCII.GetBytes("WAVE"))
        $writer.Write([System.Text.Encoding]::ASCII.GetBytes("fmt "))
        $writer.Write([int] 16)
        $writer.Write([short] 1)
        $writer.Write([short] 1)
        $writer.Write([int] $sampleRate)
        $writer.Write([int] ($sampleRate * 2))
        $writer.Write([short] 2)
        $writer.Write([short] 16)
        $writer.Write([System.Text.Encoding]::ASCII.GetBytes("data"))
        $writer.Write([int] $dataBytes)
        for ($index = 0; $index -lt $sampleCount; $index++) {
            # A deterministic low-amplitude tone avoids relying on an external
            # audio fixture while keeping the model invocation bounded.
            $sample = [int16]([math]::Round(2800 * [math]::Sin(2 * [math]::PI * 440 * $index / $sampleRate)))
            $writer.Write($sample)
        }
    }
    finally {
        $writer.Dispose()
    }
}

function Invoke-ParakeetWorkerSmoke {
    param(
        [string]$RuntimeBin,
        [string]$ModelPath,
        [string]$SmokeDir
    )

    New-Item -ItemType Directory -Force -Path $SmokeDir | Out-Null

    if (-not (Test-Path -PathType Leaf $ModelPath)) {
        throw "pinned Parakeet readiness model is missing: $ModelPath"
    }
    if ((Get-Item $ModelPath).Length -le 0) {
        throw "pinned Parakeet readiness model is empty: $ModelPath"
    }

    $wav = Join-Path $SmokeDir "readiness-tone.wav"
    $manifest = Join-Path $SmokeDir "readiness-manifest.tsv"
    $stderr = Join-Path $SmokeDir "worker.stderr.log"
    Write-MinimalPcm16Wav $wav
    [System.IO.File]::WriteAllText(
        $manifest,
        "0`t0.000`t1.000`t0.000`t1.000`t$wav`n",
        [System.Text.UTF8Encoding]::new($false)
    )

    $worker = Join-Path $RuntimeBin "parakeet-batch-json.exe"
    $rows = @(& $worker --model $ModelPath --manifest $manifest --lang auto 2> $stderr)
    $status = $LASTEXITCODE
    if ($status -ne 0) {
        $diagnostic = if (Test-Path -PathType Leaf $stderr) { Get-Content -Raw $stderr } else { "" }
        throw "parakeet-batch-json worker smoke failed with exit code ${status}: $diagnostic"
    }
    if ($rows.Count -ne 1 -or [string]::IsNullOrWhiteSpace([string]$rows[0])) {
        throw "parakeet-batch-json worker smoke expected exactly one JSON row, got $($rows.Count)"
    }
    try {
        $row = $rows[0] | ConvertFrom-Json
    }
    catch {
        throw "parakeet-batch-json worker smoke returned invalid JSON: $($_.Exception.Message)"
    }
    foreach ($property in @("index", "decode_start", "decode_end", "commit_start", "commit_end", "result")) {
        if ($null -eq $row.PSObject.Properties[$property]) {
            throw "parakeet-batch-json worker smoke row is missing '$property'"
        }
    }
    if ($row.index -ne 0 -or $row.decode_start -ne 0 -or $row.decode_end -ne 1 -or
        $row.commit_start -ne 0 -or $row.commit_end -ne 1) {
        throw "parakeet-batch-json worker smoke returned invalid chunk metadata"
    }
    if ($null -eq $row.result) {
        throw "parakeet-batch-json worker smoke returned a null result"
    }
    Write-Host "Parakeet batch worker smoke passed: exactly one JSON row from a pinned model."
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
    foreach ($binary in @("ffmpeg.exe", "whisper-cli.exe", "whisper-stream.exe", "parakeet-cli.exe", "parakeet-batch-json.exe")) {
        $path = Join-Path $runtimeBin $binary
        if (-not (Test-Path -PathType Leaf $path)) { throw "runtime is missing $binary" }
        Assert-X64Pe $path
    }
    $runtimeDlls = @(Get-ChildItem -Path $runtimeBin -File -Filter "*.dll")
    if ($runtimeDlls.Count -eq 0) { throw "Windows speech runtime contains no app-local DLLs" }
    $runtimeDlls | ForEach-Object { Assert-X64Pe $_.FullName }
    foreach ($dependency in @(
        "SDL2.dll", "whisper.dll", "ggml.dll", "ggml-base.dll", "ggml-cpu.dll",
        "parakeet.dll", "msvcp140.dll", "vcruntime140.dll", "vcruntime140_1.dll", "vcomp140.dll"
    )) {
        if (-not (Test-Path -PathType Leaf (Join-Path $runtimeBin $dependency))) {
            throw "Windows speech runtime is missing app-local dependency $dependency"
        }
    }

    $previousPath = $env:PATH
    try {
        $env:PATH = "$runtimeBin;$env:SystemRoot\System32;$env:SystemRoot"
        & (Join-Path $runtimeBin "ffmpeg.exe") -version | Out-Null
        & (Join-Path $runtimeBin "whisper-cli.exe") --help 2>&1 | Out-Null
        & (Join-Path $runtimeBin "whisper-stream.exe") --help 2>&1 | Out-Null
        $parakeetOutput = (& (Join-Path $runtimeBin "parakeet-cli.exe") --help 2>&1 | Out-String)
        if ($LASTEXITCODE -ne 0 -and $parakeetOutput -notmatch "parakeet-cli|transcribe|Usage") {
            throw "parakeet runtime probe failed: $parakeetOutput"
        }
        Invoke-ParakeetWorkerSmoke $runtimeBin $ParakeetModelPath (Join-Path $stage "parakeet-worker-smoke")
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
        & $python -c "import os, pathlib, sys; _dll=os.add_dll_directory(str(pathlib.Path(sys.executable).parent/'DLLs')); import numpy, torch, torchaudio, torchcodec; from pyannote.audio import Pipeline; print('windows-pyannote-readiness-ok')"
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
