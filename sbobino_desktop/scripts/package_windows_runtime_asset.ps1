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
$FfmpegUrl = "https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-07-10-13-44/ffmpeg-n8.1.2-22-g94138f6973-win64-gpl-shared-8.1.zip"
$FfmpegSha256 = "f5a7056f9e7e09de12fa743ed9a1802f84f644b483ca9edcc4f1d2c0fb7252ea"
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

function Find-VsTool {
    param([string]$ToolName)

    $onPath = Get-Command $ToolName -ErrorAction SilentlyContinue
    if ($onPath) { return $onPath.Source }

    $vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
    if (-not (Test-Path -PathType Leaf $vswhere)) {
        throw "Visual Studio locator was not found at '$vswhere'"
    }
    $matches = @(& $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -find "VC\Tools\MSVC\**\bin\Hostx64\x64\$ToolName")
    if ($matches.Count -eq 0) { throw "could not locate $ToolName in Visual Studio" }
    return $matches[0]
}

function Copy-VcRuntimeDependencies {
    param([string]$Destination)

    $vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
    $redistRoots = @(& $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Redist.14.Latest -find "VC\Redist\MSVC\*\x64")
    if ($redistRoots.Count -eq 0) {
        throw "could not locate the x64 Visual C++ redistributable directories"
    }

    $copied = @{}
    foreach ($redistRoot in $redistRoots) {
        foreach ($component in @("Microsoft.VC*.CRT", "Microsoft.VC*.OpenMP")) {
            Get-ChildItem -Path $redistRoot -Directory -Filter $component -ErrorAction SilentlyContinue |
                ForEach-Object {
                    Get-ChildItem -Path $_.FullName -File -Filter "*.dll" |
                        ForEach-Object {
                            Copy-Item $_.FullName (Join-Path $Destination $_.Name) -Force
                            $copied[$_.Name.ToLowerInvariant()] = $true
                        }
                }
        }
    }

    foreach ($required in @("msvcp140.dll", "vcruntime140.dll", "vcruntime140_1.dll", "vcomp140.dll")) {
        if (-not $copied.ContainsKey($required)) {
            throw "Visual C++ redistributable is missing required app-local dependency '$required'"
        }
    }
}

function Assert-AppLocalDependencies {
    param([string]$Directory)

    $dumpbin = Find-VsTool "dumpbin.exe"
    $packaged = @{}
    Get-ChildItem -Path $Directory -File | ForEach-Object {
        $packaged[$_.Name.ToLowerInvariant()] = $true
    }
    $systemDlls = @(
        "advapi32.dll", "avicap32.dll", "avrt.dll", "bcrypt.dll", "cfgmgr32.dll",
        "crypt32.dll", "d2d1.dll", "dwrite.dll", "gdi32.dll", "imm32.dll",
        "iphlpapi.dll", "kernel32.dll", "msvcrt.dll", "ncrypt.dll", "ntdll.dll",
        "ole32.dll", "oleaut32.dll", "rpcrt4.dll", "secur32.dll", "setupapi.dll",
        "shell32.dll", "shlwapi.dll", "user32.dll", "usp10.dll", "version.dll",
        "winmm.dll", "ws2_32.dll"
    )
    $system = @{}
    $systemDlls | ForEach-Object { $system[$_] = $true }

    $missing = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
    Get-ChildItem -Path (Join-Path $Directory "*") -File |
        Where-Object { $_.Extension -in @(".exe", ".dll") } |
        ForEach-Object {
        $imports = & $dumpbin /nologo /dependents $_.FullName 2>&1
        if ($LASTEXITCODE -ne 0) { throw "dumpbin dependency audit failed for '$($_.FullName)'" }
        foreach ($line in $imports) {
            if ($line -notmatch '^\s+([^\s]+\.dll)\s*$') { continue }
            $dependency = $Matches[1].ToLowerInvariant()
            if ($dependency.StartsWith("api-ms-win-") -or $dependency.StartsWith("ext-ms-win-")) { continue }
            if (-not $system.ContainsKey($dependency) -and -not $packaged.ContainsKey($dependency)) {
                [void]$missing.Add($dependency)
            }
        }
        }
    if ($missing.Count -gt 0) {
        throw "Windows runtime has non-system DLL dependencies that are not app-local: $([string]::Join(', ', @($missing)))"
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

    # Windows resolves dependencies from the executable directory before PATH.
    # Keep every native dependency app-local so clean machines do not need a
    # preinstalled VC++ runtime and never display one missing-DLL dialog per tool.
    Get-ChildItem -Path $whisperExtract -Recurse -File -Filter "*.dll" |
        ForEach-Object { Copy-Item $_.FullName (Join-Path $binDir $_.Name) -Force }
    Get-ChildItem -Path $ffmpegExtract -Recurse -File -Filter "*.dll" |
        ForEach-Object { Copy-Item $_.FullName (Join-Path $binDir $_.Name) -Force }
    Copy-VcRuntimeDependencies $binDir
    "Native DLLs are deployed app-local in runtime/bin on Windows." |
        Set-Content -Encoding UTF8 (Join-Path $libDir "README.txt")

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
    Get-ChildItem -Path $binDir -File -Filter "*.dll" |
        ForEach-Object { Assert-X64Pe $_.FullName }
    Assert-AppLocalDependencies $binDir

    $previousPath = $env:PATH
    try {
        $env:PATH = "$binDir;$env:SystemRoot\System32;$env:SystemRoot"
        & (Join-Path $binDir "ffmpeg.exe") -version | Out-Null
        & (Join-Path $binDir "whisper-cli.exe") --help 2>&1 | Out-Null
        & (Join-Path $binDir "whisper-stream.exe") --help 2>&1 | Out-Null
        $parakeetOutput = (& (Join-Path $binDir "parakeet-cli.exe") --help 2>&1 | Out-String)
        if ($LASTEXITCODE -ne 0 -and $parakeetOutput -notmatch "parakeet-cli|transcribe|Usage") {
            throw "parakeet-cli probe failed: $parakeetOutput"
        }
        $global:LASTEXITCODE = 0
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
$global:LASTEXITCODE = 0
