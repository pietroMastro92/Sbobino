param(
    [Parameter(Mandatory = $true)]
    [string]$OutputZip,
    [string]$SidecarDir = ""
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$WhisperUrl = "https://github.com/ggml-org/whisper.cpp/releases/download/v1.8.4/whisper-bin-x64.zip"
$WhisperSha256 = "74f973345cb52ef5ba3ec9e7e7af8e48cc8c71722d1528603b80588a11f82e3e"
$ParakeetUrl = "https://github.com/mudler/parakeet.cpp/releases/download/v0.4.0/parakeet-v0.4.0-bin-win-cpu-x64.zip"
$ParakeetSha256 = "2880150a1bad2944baed46f2e6bb9f1bc55263a9f2bb85573785a7ec4fa35f27"
$FfmpegUrl = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-n8.1-latest-win64-gpl-shared-8.1.zip"
$FfmpegSha256 = "769e77776cee6530595d75271b3d474d95af98b0bd1d1a8c83c28f633b78d619"
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

function Find-OneFile {
    param([string]$Root, [string]$Name)
    $matches = @(Get-ChildItem -Path $Root -Recurse -File -Filter $Name)
    if ($matches.Count -ne 1) {
        throw "expected exactly one '$Name' below '$Root', found $($matches.Count)"
    }
    return $matches[0].FullName
}

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

$stage = Join-Path ([System.IO.Path]::GetTempPath()) ("sbobino-runtime-" + [guid]::NewGuid())
$downloads = Join-Path $stage "downloads"
$extract = Join-Path $stage "extract"
$runtimeRoot = Join-Path $stage "runtime"
$binDir = Join-Path $runtimeRoot "bin"
$libDir = Join-Path $runtimeRoot "lib"

try {
    New-Item -ItemType Directory -Force -Path $downloads, $extract, $binDir, $libDir | Out-Null

    $whisperArchive = Join-Path $downloads "whisper.zip"
    $parakeetArchive = Join-Path $downloads "parakeet.zip"
    $ffmpegArchive = Join-Path $downloads "ffmpeg.zip"
    Download-VerifiedArchive $WhisperUrl $whisperArchive $WhisperSha256
    Download-VerifiedArchive $ParakeetUrl $parakeetArchive $ParakeetSha256
    Download-VerifiedArchive $FfmpegUrl $ffmpegArchive $FfmpegSha256

    $whisperExtract = Join-Path $extract "whisper"
    $parakeetExtract = Join-Path $extract "parakeet"
    $ffmpegExtract = Join-Path $extract "ffmpeg"
    Expand-Archive -Path $whisperArchive -DestinationPath $whisperExtract
    Expand-Archive -Path $parakeetArchive -DestinationPath $parakeetExtract
    Expand-Archive -Path $ffmpegArchive -DestinationPath $ffmpegExtract

    Copy-Item (Find-OneFile $whisperExtract "whisper-cli.exe") (Join-Path $binDir "whisper-cli.exe")
    Copy-Item (Find-OneFile $whisperExtract "whisper-stream.exe") (Join-Path $binDir "whisper-stream.exe")
    Copy-Item (Find-OneFile $parakeetExtract "parakeet-cli.exe") (Join-Path $binDir "parakeet-cli.exe")
    Copy-Item (Find-OneFile $ffmpegExtract "ffmpeg.exe") (Join-Path $binDir "ffmpeg.exe")

    Get-ChildItem -Path $whisperExtract -Recurse -File -Filter "*.dll" |
        ForEach-Object { Copy-Item $_.FullName (Join-Path $libDir $_.Name) -Force }
    Get-ChildItem -Path $ffmpegExtract -Recurse -File -Filter "*.dll" |
        ForEach-Object { Copy-Item $_.FullName (Join-Path $libDir $_.Name) -Force }

    @(
        "runtime_arch=$TargetTriple"
        "whisper_cpp_version=1.8.4"
        "parakeet_cpp_version=0.4.0"
        "ffmpeg_version=8.1"
        "parakeet_backend=cpu"
    ) | Set-Content -Encoding UTF8 (Join-Path $binDir "runtime-manifest.txt")

    foreach ($binary in @("ffmpeg.exe", "whisper-cli.exe", "whisper-stream.exe", "parakeet-cli.exe")) {
        Assert-X64Pe (Join-Path $binDir $binary)
    }
    Get-ChildItem -Path $libDir -File -Filter "*.dll" |
        ForEach-Object { Assert-X64Pe $_.FullName }

    $previousPath = $env:PATH
    try {
        $env:PATH = "$binDir;$libDir;$env:SystemRoot\System32"
        & (Join-Path $binDir "ffmpeg.exe") -version | Out-Null
        & (Join-Path $binDir "whisper-cli.exe") --help 2>&1 | Out-Null
        & (Join-Path $binDir "whisper-stream.exe") --help 2>&1 | Out-Null
        $parakeetOutput = (& (Join-Path $binDir "parakeet-cli.exe") --help 2>&1 | Out-String)
        if ($LASTEXITCODE -ne 0 -and $parakeetOutput -notmatch "parakeet-cli|transcribe|Usage") {
            throw "parakeet-cli probe failed: $parakeetOutput"
        }
    }
    finally {
        $env:PATH = $previousPath
    }

    if ($SidecarDir) {
        New-Item -ItemType Directory -Force -Path $SidecarDir | Out-Null
        $sidecars = @{
            "ffmpeg" = "ffmpeg.exe"
            "whisper-cli" = "whisper-cli.exe"
            "whisperkit-cli" = "whisper-cli.exe"
            "whisper-stream" = "whisper-stream.exe"
            "parakeet-cli" = "parakeet-cli.exe"
        }
        foreach ($name in $sidecars.Keys) {
            Copy-Item (Join-Path $binDir $sidecars[$name]) `
                (Join-Path $SidecarDir "$name-$TargetTriple.exe") -Force
        }
    }

    $outputParent = Split-Path -Parent $OutputZip
    if ($outputParent) { New-Item -ItemType Directory -Force -Path $outputParent | Out-Null }
    Remove-Item -Force -ErrorAction SilentlyContinue $OutputZip
    Compress-Archive -Path $runtimeRoot -DestinationPath $OutputZip -CompressionLevel Optimal
    Write-Host "Created native Windows speech runtime: $OutputZip"
}
finally {
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $stage
}
