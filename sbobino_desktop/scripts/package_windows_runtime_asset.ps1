param(
    [Parameter(Mandatory = $true)]
    [string]$OutputZip,
    [string]$SidecarDir = ""
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$WhisperSourceUrl = "https://github.com/ggml-org/whisper.cpp.git"
$WhisperSourceRef = "9386f239401074690479731c1e41683fbbeac557"
$SdlVersion = "2.32.10"
$SdlUrl = "https://github.com/libsdl-org/SDL/releases/download/release-$SdlVersion/SDL2-devel-$SdlVersion-VC.zip"
$SdlSha256 = "af347939395a58b365846aaea27391e69f9ec9d4dd650d6ac40802159b418a6e"
$ParakeetVersion = "0.4.0"
$ParakeetUrl = "https://github.com/mudler/parakeet.cpp/releases/download/v$ParakeetVersion/parakeet-v$ParakeetVersion-bin-win-cpu-x64.zip"
$ParakeetSha256 = "2880150a1bad2944baed46f2e6bb9f1bc55263a9f2bb85573785a7ec4fa35f27"
$ParakeetSourceUrl = "https://github.com/mudler/parakeet.cpp.git"
$ParakeetSourceRef = "fa5aeef1e3d353679cbd374a426fee28387deb6e"
# BtbN prunes dated autobuild releases, so a URL pinned to one of those tags is
# not durable. Reuse the immutable, already validated Windows runtime from the
# immediately preceding stable release, which the promotion policy retains.
$FfmpegUrl = "https://github.com/pietroMastro92/Sbobino/releases/download/v2.0.25/speech-runtime-windows-x86_64.zip"
$FfmpegSha256 = "a0dadc1a7a58008a4c45b9c612a1c58ecc2a85baa6eefecad083f78c746a8c83"
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

function Find-VsDevCmd {
    $onPath = Get-Command "VsDevCmd.bat" -ErrorAction SilentlyContinue
    if ($onPath) { return $onPath.Source }

    $vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
    if (-not (Test-Path -PathType Leaf $vswhere)) {
        throw "Visual Studio locator was not found at '$vswhere'"
    }
    $installationPath = @(& $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath)
    if ($installationPath.Count -eq 0) {
        throw "could not locate a Visual Studio installation with x64 C++ tools"
    }
    $vsDevCmd = Join-Path $installationPath[0] "Common7\Tools\VsDevCmd.bat"
    if (-not (Test-Path -PathType Leaf $vsDevCmd)) {
        throw "Visual Studio developer command file was not found at '$vsDevCmd'"
    }
    return $vsDevCmd
}

function Invoke-VsCommand {
    param([string]$CommandLine)

    $vsDevCmd = Find-VsDevCmd
    $quotedVsDevCmd = '"' + $vsDevCmd.Replace('"', '\"') + '"'
    & cmd.exe /d /s /c "$quotedVsDevCmd -arch=x64 -host_arch=x64 >nul && $CommandLine"
    if ($LASTEXITCODE -ne 0) {
        throw "Visual Studio command failed with exit code ${LASTEXITCODE}: $CommandLine"
    }
}

function Find-CMakeVisualStudioGenerator {
    $cmake = Get-Command "cmake.exe" -ErrorAction SilentlyContinue
    if (-not $cmake) {
        throw "CMake was not found on PATH while discovering a Visual Studio generator"
    }

    $capabilitiesOutput = @(& $cmake.Source -E capabilities)
    if ($LASTEXITCODE -ne 0 -or $capabilitiesOutput.Count -eq 0) {
        throw "cmake -E capabilities failed while discovering a Visual Studio generator"
    }

    try {
        $capabilities = ($capabilitiesOutput -join "`n") | ConvertFrom-Json
    }
    catch {
        throw "cmake -E capabilities returned invalid JSON while discovering a Visual Studio generator: $($_.Exception.Message)"
    }

    $visualStudioGenerators = @(
        @(
            foreach ($generator in @($capabilities.generators)) {
                $match = [regex]::Match(
                    [string]$generator.name,
                    '^Visual Studio (?<version>\d+) (?<year>\d{4})$'
                )
                if ($match.Success -and $generator.platformSupport) {
                    [pscustomobject]@{
                        Name = [string]$generator.name
                        Version = [int]$match.Groups['version'].Value
                        Year = [int]$match.Groups['year'].Value
                    }
                }
            }
        ) | Sort-Object Version, Year -Descending
    )

    if ($visualStudioGenerators.Count -eq 0) {
        $available = @($capabilities.generators | ForEach-Object { [string]$_.name }) -join ", "
        throw "CMake exposes no supported Visual Studio generator; available generators: $available"
    }

    return $visualStudioGenerators[0].Name
}

function Checkout-ParakeetSource {
    param([string]$Destination)

    & git clone --no-checkout --filter=blob:none --recurse-submodules $ParakeetSourceUrl $Destination
    if ($LASTEXITCODE -ne 0) {
        throw "failed to clone pinned Parakeet source from $ParakeetSourceUrl"
    }
    & git -C $Destination checkout --detach $ParakeetSourceRef
    if ($LASTEXITCODE -ne 0) {
        throw "failed to checkout pinned Parakeet source ref $ParakeetSourceRef"
    }
    & git -C $Destination submodule update --init --recursive
    if ($LASTEXITCODE -ne 0) {
        throw "failed to initialize pinned Parakeet source submodules"
    }
    $resolved = (& git -C $Destination rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0 -or $resolved -ne $ParakeetSourceRef) {
        throw "pinned Parakeet source resolved to '$resolved', expected '$ParakeetSourceRef'"
    }
}

function Checkout-WhisperSource {
    param([string]$Destination)

    & git clone --filter=blob:none $WhisperSourceUrl $Destination
    if ($LASTEXITCODE -ne 0) {
        throw "failed to clone pinned Whisper source from $WhisperSourceUrl"
    }
    & git -C $Destination checkout --detach $WhisperSourceRef
    if ($LASTEXITCODE -ne 0) {
        throw "failed to checkout pinned Whisper source ref $WhisperSourceRef"
    }
    $resolved = (& git -C $Destination rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0 -or $resolved -ne $WhisperSourceRef) {
        throw "pinned Whisper source resolved to '$resolved', expected '$WhisperSourceRef'"
    }
    foreach ($patchName in @(
        "whisper-stream-audio-file.patch",
        "whisper-stream-fifo.patch",
        "whisper-stream-backlog.patch",
        "whisper-stream-finalization.patch",
        "whisper-stream-lossless-drain.patch"
    )) {
        $patchPath = Join-Path $PSScriptRoot "patches\$patchName"
        & git -C $Destination apply --check $patchPath
        if ($LASTEXITCODE -ne 0) { throw "Whisper patch check failed: $patchName" }
        & git -C $Destination apply $patchPath
        if ($LASTEXITCODE -ne 0) { throw "Whisper patch apply failed: $patchName" }
    }
}

function Find-X64SdlFile {
    param([string]$Root, [string]$Name)

    $escapedName = [regex]::Escape($Name)
    $matches = @(
        Get-ChildItem -Path $Root -Recurse -File -Filter $Name |
            Where-Object { $_.FullName -match "[\\/]lib[\\/]x64[\\/]${escapedName}$" }
    )
    if ($matches.Count -ne 1) {
        throw "expected exactly one x64 '$Name' below '$Root', found $($matches.Count)"
    }
    return $matches[0].FullName
}

function Find-SdlCMakePackage {
    param([string]$SdlRoot)

    # The official VC archive ships its package as sdl2-config.cmake (with a
    # hyphen), not SDL2Config.cmake. It defines SDL2::SDL2main before
    # SDL2::SDL2, which is required for whisper-stream's Windows entry point.
    $configPath = Find-OneFile $SdlRoot "sdl2-config.cmake"
    Find-OneFile $SdlRoot "SDL.h" | Out-Null
    Find-X64SdlFile $SdlRoot "SDL2main.lib" | Out-Null
    Find-X64SdlFile $SdlRoot "SDL2.lib" | Out-Null
    return (Split-Path -Parent $configPath)
}

function Build-WhisperBinaries {
    param(
        [string]$SourceDir,
        [string]$BuildDir,
        [string]$SdlRoot,
        [string]$Destination
    )

    New-Item -ItemType Directory -Force -Path $BuildDir | Out-Null
    $generatorName = Find-CMakeVisualStudioGenerator
    $generator = Quote-CmdArg $generatorName
    $sourceArg = Quote-CmdArg $SourceDir
    $buildArg = Quote-CmdArg $BuildDir
    $sdlCmakeDir = Find-SdlCMakePackage $SdlRoot
    Invoke-VsCommand ("cmake.exe -S {0} -B {1} -G {2} -A x64 -DCMAKE_BUILD_TYPE=Release -DSDL2_DIR={3} -DBUILD_SHARED_LIBS=ON -DWHISPER_BUILD_EXAMPLES=ON -DWHISPER_BUILD_TESTS=OFF -DWHISPER_BUILD_SERVER=OFF -DWHISPER_SDL2=ON -DGGML_NATIVE=OFF" -f `
        $sourceArg, $buildArg, $generator, (Quote-CmdArg $sdlCmakeDir))
    Invoke-VsCommand ("cmake.exe --build {0} --config Release --target whisper-cli whisper-stream" -f $buildArg)

    Copy-Item (Find-OneFile $BuildDir "whisper-cli.exe") (Join-Path $Destination "whisper-cli.exe") -Force
    Copy-Item (Find-OneFile $BuildDir "whisper-stream.exe") (Join-Path $Destination "whisper-stream.exe") -Force
    Get-ChildItem -Path $BuildDir -Recurse -File -Filter "*.dll" |
        Sort-Object Name, FullName |
        ForEach-Object { Copy-Item $_.FullName (Join-Path $Destination $_.Name) -Force }
    $sdlDll = @(Get-ChildItem -Path $SdlRoot -Recurse -File -Filter "SDL2.dll" |
        Where-Object { $_.FullName -match '[\\/]lib[\\/]x64[\\/]SDL2\.dll$' })
    if ($sdlDll.Count -ne 1) {
        throw "expected exactly one x64 SDL2.dll below '$SdlRoot', found $($sdlDll.Count)"
    }
    Copy-Item $sdlDll[0].FullName (Join-Path $Destination "SDL2.dll") -Force
}

function Build-ParakeetSharedLibrary {
    param(
        [string]$SourceDir,
        [string]$BuildDir
    )

    New-Item -ItemType Directory -Force -Path $BuildDir | Out-Null
    $generatorName = Find-CMakeVisualStudioGenerator
    $generator = Quote-CmdArg $generatorName
    $sourceArg = Quote-CmdArg $SourceDir
    $buildArg = Quote-CmdArg $BuildDir
    Invoke-VsCommand ("cmake.exe -S {0} -B {1} -G {2} -A x64 -DCMAKE_BUILD_TYPE=Release -DPARAKEET_SHARED=ON -DPARAKEET_BUILD_CLI=ON -DPARAKEET_BUILD_SERVER=OFF -DPARAKEET_BUILD_TESTS=OFF -DGGML_NATIVE=OFF -DPARAKEET_GGML_METAL=OFF -DCMAKE_WINDOWS_EXPORT_ALL_SYMBOLS=ON" -f `
        $sourceArg, $buildArg, $generator)
    Invoke-VsCommand ("cmake.exe --build {0} --config Release --target parakeet" -f $buildArg)
}

function Copy-VcRuntimeDependencies {
    param([string]$Destination)

    $vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
    $redistRoots = @(& $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Redist.14.Latest -find "VC\Redist\MSVC\*\x64") |
        Sort-Object
    if ($redistRoots.Count -eq 0) {
        throw "could not locate the x64 Visual C++ redistributable directories"
    }

    $copied = @{}
    foreach ($redistRoot in $redistRoots) {
        foreach ($component in @("Microsoft.VC*.CRT", "Microsoft.VC*.OpenMP")) {
            Get-ChildItem -Path $redistRoot -Directory -Filter $component -ErrorAction SilentlyContinue |
                Sort-Object FullName |
                ForEach-Object {
                    Get-ChildItem -Path $_.FullName -File -Filter "*.dll" |
                        Sort-Object Name, FullName |
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
        "shell32.dll", "shlwapi.dll", "ucrtbase.dll", "user32.dll", "usp10.dll", "version.dll",
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

function Quote-CmdArg {
    param([string]$Value)
    return '"' + $Value.Replace('"', '\"') + '"'
}

function Build-ParakeetBatchWorker {
    param(
        [string]$SourcePath,
        [string]$IncludeDir,
        [string]$ParakeetDll,
        [string]$BuildDir,
        [string]$OutputPath
    )

    New-Item -ItemType Directory -Force -Path $BuildDir | Out-Null
    $dumpbin = Find-VsTool "dumpbin.exe"
    $exports = @(& $dumpbin /nologo /exports $ParakeetDll 2>&1)
    if ($LASTEXITCODE -ne 0) {
        throw "dumpbin export audit failed for '$ParakeetDll'"
    }

    $requiredExports = @(
        "parakeet_capi_load",
        "parakeet_capi_free",
        "parakeet_capi_transcribe_path_json",
        "parakeet_capi_transcribe_pcm_batch_json_lang",
        "parakeet_capi_free_string",
        "parakeet_capi_last_error"
    )
    foreach ($name in $requiredExports) {
        if (-not ($exports | Where-Object { $_ -match "\s$name\s*$" })) {
            throw "Parakeet DLL '$ParakeetDll' does not export required C-API symbol '$name'"
        }
    }

    $defPath = Join-Path $BuildDir "parakeet.def"
    @(
        "LIBRARY parakeet.dll"
        "EXPORTS"
        $requiredExports
    ) | Set-Content -Encoding ASCII $defPath

    $importLib = Join-Path $BuildDir "parakeet.lib"
    $objectPath = Join-Path $BuildDir "parakeet_batch_json.obj"
    Invoke-VsCommand ("lib.exe /nologo /def:{0} /machine:x64 /out:{1}" -f `
        (Quote-CmdArg $defPath), (Quote-CmdArg $importLib))
    Invoke-VsCommand ("cl.exe /nologo /std:c++17 /EHsc /MD /O2 /W4 /I{0} /c {1} /Fo{2}" -f `
        (Quote-CmdArg $IncludeDir), (Quote-CmdArg $SourcePath), (Quote-CmdArg $objectPath))
    Invoke-VsCommand ("link.exe /nologo /machine:x64 /subsystem:console /out:{0} {1} {2}" -f `
        (Quote-CmdArg $OutputPath), (Quote-CmdArg $objectPath), (Quote-CmdArg $importLib))
    Assert-X64Pe $OutputPath
}

$stage = Join-Path ([System.IO.Path]::GetTempPath()) ("sbobino-runtime-" + [guid]::NewGuid())
$downloads = Join-Path $stage "downloads"
$extract = Join-Path $stage "extract"
$parakeetSource = Join-Path $stage "parakeet-source"
$parakeetBuild = Join-Path $stage "parakeet-build"
$whisperSource = Join-Path $stage "whisper-source"
$whisperBuild = Join-Path $stage "whisper-build"
$runtimeRoot = Join-Path $stage "runtime"
$binDir = Join-Path $runtimeRoot "bin"
$libDir = Join-Path $runtimeRoot "lib"

try {
    New-Item -ItemType Directory -Force -Path $downloads, $extract, $binDir, $libDir | Out-Null

    $sdlArchive = Join-Path $downloads "sdl2.zip"
    $parakeetArchive = Join-Path $downloads "parakeet.zip"
    $ffmpegArchive = Join-Path $downloads "ffmpeg.zip"
    Download-VerifiedArchive $SdlUrl $sdlArchive $SdlSha256
    Download-VerifiedArchive $ParakeetUrl $parakeetArchive $ParakeetSha256
    Download-VerifiedArchive $FfmpegUrl $ffmpegArchive $FfmpegSha256

    $sdlExtract = Join-Path $extract "sdl2"
    $parakeetExtract = Join-Path $extract "parakeet"
    $ffmpegExtract = Join-Path $extract "ffmpeg"
    Expand-Archive -Path $sdlArchive -DestinationPath $sdlExtract
    Expand-Archive -Path $parakeetArchive -DestinationPath $parakeetExtract
    Expand-Archive -Path $ffmpegArchive -DestinationPath $ffmpegExtract

    Checkout-WhisperSource $whisperSource
    Build-WhisperBinaries $whisperSource $whisperBuild $sdlExtract $binDir
    Copy-Item (Find-OneFile $parakeetExtract "parakeet-cli.exe") (Join-Path $binDir "parakeet-cli.exe")
    Copy-Item (Find-OneFile $ffmpegExtract "ffmpeg.exe") (Join-Path $binDir "ffmpeg.exe")

    # The official v0.4.0 Windows archive intentionally contains only the CLI
    # and server executables. Build the shared C-API library from the exact
    # pinned upstream source instead of depending on a non-existent "lib"
    # archive, then link the batch worker against the resulting import library.
    Checkout-ParakeetSource $parakeetSource
    Build-ParakeetSharedLibrary $parakeetSource $parakeetBuild
    $parakeetDll = Find-OneFile $parakeetBuild "parakeet.dll"
    $parakeetHeader = Find-OneFile $parakeetSource "parakeet_capi.h"
    Copy-Item $parakeetDll (Join-Path $binDir "parakeet.dll")
    # Keep the C-API library in runtime/lib for the desktop app while the
    # loader also resolves a copy next to the worker on Windows.
    Copy-Item $parakeetDll (Join-Path $libDir "parakeet.dll")

    $workerBuildDir = Join-Path $stage "parakeet-worker-build"
    $workerPath = Join-Path $binDir "parakeet-batch-json.exe"
    Build-ParakeetBatchWorker `
        (Join-Path $PSScriptRoot "parakeet_batch_json.cpp") `
        (Split-Path -Parent $parakeetHeader) `
        (Join-Path $binDir "parakeet.dll") `
        $workerBuildDir `
        $workerPath

    # Windows resolves dependencies from the executable directory before PATH.
    # Keep every native dependency app-local so clean machines do not need a
    # preinstalled VC++ runtime and never display one missing-DLL dialog per tool.
    Get-ChildItem -Path $ffmpegExtract -Recurse -File -Filter "*.dll" |
        Sort-Object Name, FullName |
        ForEach-Object { Copy-Item $_.FullName (Join-Path $binDir $_.Name) -Force }
    Copy-VcRuntimeDependencies $binDir
    "Native DLLs are deployed app-local in runtime/bin on Windows." |
        Set-Content -Encoding UTF8 (Join-Path $libDir "README.txt")

    @(
        "runtime_arch=$TargetTriple"
        "whisper_cpp_version=1.8.4"
        "whisper_cpp_source_ref=$WhisperSourceRef"
        "parakeet_cpp_version=0.4.0"
        "parakeet_cpp_source_ref=$ParakeetSourceRef"
        "ffmpeg_version=8.1"
        "parakeet_backend=cpu"
    ) | Set-Content -Encoding UTF8 (Join-Path $binDir "runtime-manifest.txt")

    foreach ($binary in @("ffmpeg.exe", "whisper-cli.exe", "whisper-stream.exe", "parakeet-cli.exe", "parakeet-batch-json.exe")) {
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
            "parakeet-batch-json" = "parakeet-batch-json.exe"
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
