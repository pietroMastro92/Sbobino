param(
    [Parameter(Mandatory = $true)]
    [string]$Version,
    [string]$RepoSlug = "pietroMastro92/Sbobino",
    [string]$ReportPath = "WINDOWS-PRIMARY.validation-report.json",
    [string]$FixtureAudio = "",
    [string]$AppPath = "",
    [string]$DataDir = "",
    [int]$TimeoutSeconds = 2400,
    [string]$PrivacyPolicyVersion = "2026-04-03",
    [string]$ParakeetModel = "tdt-0.6b-v3-q4_k.gguf",
    [string]$RunnerLabel = "github-hosted,windows-2025,windows-primary"
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$MachineClass = "WINDOWS-PRIMARY"
$Tag = "v$Version"
$ReleaseUrl = "https://github.com/$RepoSlug/releases/tag/$Tag"
$BaseDownloadUrl = "https://github.com/$RepoSlug/releases/download/$Tag"
$ScriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = Split-Path -Parent $ScriptRoot
$CommitSha = if ($env:GITHUB_SHA) { $env:GITHUB_SHA } else { "" }
$Tester = if ($env:GITHUB_ACTOR) { $env:GITHUB_ACTOR } else { $env:USERNAME }
$OsName = "Windows"
$OsVersion = [System.Environment]::OSVersion.VersionString

if (-not $AppPath) {
    $AppPath = Join-Path $env:LOCALAPPDATA "Sbobino\sbobino-desktop.exe"
}
if (-not $DataDir) {
    $DataDir = Join-Path $env:APPDATA "com.sbobino.desktop"
}

$SetupReportPath = Join-Path $DataDir "setup-report.json"
$SettingsPath = Join-Path $DataDir "settings.json"
$TmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ("sbobino-windows-primary-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $TmpDir | Out-Null

$FinalStatus = "failed"
$ReportNotes = ""
$ScenarioResults = [ordered]@{
    clean_room_install              = "pending"
    first_setup                     = "pending"
    functional_transcription_smoke  = "pending"
    functional_diarization_smoke    = "pending"
    warm_restart                    = "pending"
    no_visible_console_windows      = "pending"
    opaque_main_window              = "pending"
}
$RequiredScenarios = @(
    "clean_room_install",
    "first_setup",
    "functional_transcription_smoke",
    "functional_diarization_smoke",
    "warm_restart",
    "no_visible_console_windows",
    "opaque_main_window"
)

function Write-ValidationReport {
    $payload = [ordered]@{
        schema_version      = 1
        version             = $Version
        release_tag         = $Tag
        release_url         = $ReleaseUrl
        commit_sha          = $CommitSha
        machine_class       = $MachineClass
        status              = $FinalStatus
        tester              = $Tester
        os_name             = $OsName
        os_version          = $OsVersion
        runner_label        = $RunnerLabel
        tested_at_utc       = [DateTime]::UtcNow.ToString("yyyy-MM-ddTHH:mm:ssZ")
        notes               = $ReportNotes
        required_scenarios  = $RequiredScenarios
        scenario_results    = $ScenarioResults
    }
    $reportDirectory = Split-Path -Parent $ReportPath
    if ($reportDirectory) {
        New-Item -ItemType Directory -Force -Path $reportDirectory | Out-Null
    }
    $payload | ConvertTo-Json -Depth 8 | Set-Content -Encoding UTF8 $ReportPath
}

function Fail-Validation {
    param([string]$Message)
    $script:FinalStatus = "failed"
    $script:ReportNotes = $Message
    Write-ValidationReport
    throw $Message
}

function Download-ReleaseAsset {
    param(
        [string]$AssetName,
        [string]$Destination
    )
    $url = "$BaseDownloadUrl/$AssetName"
    Invoke-WebRequest -Uri $url -OutFile $Destination -UseBasicParsing
    if (-not (Test-Path -PathType Leaf $Destination) -or (Get-Item $Destination).Length -le 0) {
        Fail-Validation "Failed to download release asset '$AssetName'."
    }
}

function Get-Sha256Hex {
    param([string]$Path)
    return (Get-FileHash -Algorithm SHA256 -Path $Path).Hash.ToLowerInvariant()
}

function Stop-SbobinoProcesses {
    Get-Process -Name "sbobino-desktop" -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 2
}

function Clear-InstallState {
    Stop-SbobinoProcesses
    $uninstaller = Join-Path $env:LOCALAPPDATA "Sbobino\uninstall.exe"
    if (Test-Path -PathType Leaf $uninstaller) {
        $proc = Start-Process -FilePath $uninstaller -ArgumentList "/S" -Wait -PassThru
        if ($proc.ExitCode -ne 0) {
            Write-Host "Uninstaller exited with $($proc.ExitCode); continuing with forced cleanup."
        }
    }
    Stop-SbobinoProcesses
    $installDir = Split-Path -Parent $AppPath
    if (Test-Path $installDir) {
        Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $installDir
    }
    if (Test-Path $DataDir) {
        Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $DataDir
    }
}

function Invoke-PythonSettingsMutator {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Code,
        [Parameter(Mandatory = $true)]
        [string[]]$PythonArgs
    )
    $python = Get-Command python -ErrorAction SilentlyContinue
    if (-not $python) {
        $python = Get-Command python3 -ErrorAction SilentlyContinue
    }
    if (-not $python) {
        Fail-Validation "Python is required to mutate Windows validation settings.json."
    }
    $scriptFile = Join-Path $TmpDir ("settings-mutator-" + [guid]::NewGuid().ToString("N") + ".py")
    Set-Content -Path $scriptFile -Value $Code -Encoding UTF8
    & $python.Source $scriptFile @PythonArgs
    if ($LASTEXITCODE -ne 0) {
        Fail-Validation "Failed to update Windows validation settings.json."
    }
}

function Seed-PrivacyAcceptance {
    New-Item -ItemType Directory -Force -Path $DataDir | Out-Null
    $code = @'
import json
import sys
from datetime import datetime, timezone
from pathlib import Path

settings_path = Path(sys.argv[1])
privacy_version = sys.argv[2]
payload = {}
if settings_path.exists():
    try:
        payload = json.loads(settings_path.read_text(encoding="utf-8"))
    except Exception:
        payload = {}
general = payload.setdefault("general", {})
general["privacy_policy_version_accepted"] = privacy_version
general["privacy_policy_accepted_at"] = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
settings_path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
'@
    Invoke-PythonSettingsMutator -Code $code -PythonArgs @($SettingsPath, $PrivacyPolicyVersion)
}

function Set-SpeakerDiarizationEnabled {
    param([bool]$Enabled)
    New-Item -ItemType Directory -Force -Path $DataDir | Out-Null
    $flag = if ($Enabled) { "1" } else { "0" }
    $code = @'
import json
import sys
from pathlib import Path

settings_path = Path(sys.argv[1])
enabled = sys.argv[2] == "1"
payload = {}
if settings_path.exists():
    try:
        payload = json.loads(settings_path.read_text(encoding="utf-8"))
    except Exception:
        payload = {}
transcription = payload.setdefault("transcription", {})
speaker = transcription.setdefault("speaker_diarization", {})
speaker["enabled"] = enabled
speaker.setdefault("device", "cpu")
speaker.setdefault("speaker_colors", {})
settings_path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
'@
    Invoke-PythonSettingsMutator -Code $code -PythonArgs @($SettingsPath, $flag)
}

function Launch-App {
    if (-not (Test-Path -PathType Leaf $AppPath)) {
        Fail-Validation "Installed Sbobino executable not found at '$AppPath'."
    }
    Start-Process -FilePath $AppPath | Out-Null
}

function Wait-ForSetupReportSuccess {
    param([int]$Timeout)
    $started = Get-Date
    while ($true) {
        if (Test-Path -PathType Leaf $SetupReportPath) {
            try {
                $report = Get-Content -Raw $SetupReportPath | ConvertFrom-Json
                $setupComplete = $report.setup_complete -eq $true
                $reason = [string]$report.final_reason_code
                $errorText = [string]$report.final_error
                if ($setupComplete -and $reason.Trim() -eq "setup_complete" -and -not $errorText.Trim()) {
                    return
                }
                $steps = @($report.steps)
                if ($steps | Where-Object { [string]$_.status -eq "failed" }) {
                    Fail-Validation "setup-report.json indicates a failed first-launch setup: $(Get-Content -Raw $SetupReportPath)"
                }
                if ($errorText.Trim()) {
                    Fail-Validation "setup-report.json indicates a failed first-launch setup: $(Get-Content -Raw $SetupReportPath)"
                }
            }
            catch {
                # still writing
            }
        }
        if (((Get-Date) - $started).TotalSeconds -gt $Timeout) {
            $body = if (Test-Path $SetupReportPath) { Get-Content -Raw $SetupReportPath } else { "<missing>" }
            Fail-Validation "Timed out waiting for setup-report.json to report setup_complete. Last report: $body"
        }
        Start-Sleep -Seconds 10
    }
}

function Wait-ForManagedRuntimeReady {
    param(
        [int]$Timeout,
        [bool]$RequirePyannote
    )
    $started = Get-Date
    while ($true) {
        $parakeet = Join-Path $DataDir "bin\parakeet-cli.exe"
        $ffmpeg = Join-Path $DataDir "bin\ffmpeg.exe"
        $modelsDir = Join-Path $DataDir "parakeet-models"
        $modelPath = Join-Path $modelsDir $ParakeetModel
        $runtimeReady = (Test-Path -PathType Leaf $parakeet) -and (Test-Path -PathType Leaf $ffmpeg) -and (Test-Path -PathType Leaf $modelPath)
        $pyannoteReady = $true
        if ($RequirePyannote) {
            $python = Join-Path $DataDir "runtime\pyannote\python\python.exe"
            $modelDir = Join-Path $DataDir "runtime\pyannote\model"
            $pyannoteReady = (Test-Path -PathType Leaf $python) -and (Test-Path -PathType Container $modelDir)
        }
        if ($runtimeReady -and $pyannoteReady) {
            return
        }
        if (((Get-Date) - $started).TotalSeconds -gt $Timeout) {
            Fail-Validation "Timed out waiting for managed Windows runtime readiness (require_pyannote=$RequirePyannote)."
        }
        Start-Sleep -Seconds 10
    }
}

function Install-PyannoteReleaseAssets {
    $runtimeDir = Join-Path $DataDir "runtime\pyannote"
    $manifestPath = Join-Path $TmpDir "pyannote-manifest.json"
    Download-ReleaseAsset -AssetName "pyannote-manifest.json" -Destination $manifestPath
    $manifest = Get-Content -Raw $manifestPath | ConvertFrom-Json
    $runtimeAsset = @($manifest.assets | Where-Object { $_.kind -eq "pyannote_runtime_windows_x86_64" }) | Select-Object -First 1
    $modelAsset = @($manifest.assets | Where-Object { $_.kind -eq "pyannote_model" }) | Select-Object -First 1
    if ($null -eq $runtimeAsset -or $null -eq $modelAsset) {
        Fail-Validation "pyannote-manifest.json is missing Windows runtime or model asset metadata."
    }

    $runtimeZip = Join-Path $TmpDir $runtimeAsset.name
    $modelZip = Join-Path $TmpDir $modelAsset.name
    Download-ReleaseAsset -AssetName $runtimeAsset.name -Destination $runtimeZip
    Download-ReleaseAsset -AssetName $modelAsset.name -Destination $modelZip

    $runtimeSha = Get-Sha256Hex $runtimeZip
    $modelSha = Get-Sha256Hex $modelZip
    if ($runtimeSha -ne ([string]$runtimeAsset.sha256).ToLowerInvariant()) {
        Fail-Validation "Checksum mismatch for '$($runtimeAsset.name)'."
    }
    if ($modelSha -ne ([string]$modelAsset.sha256).ToLowerInvariant()) {
        Fail-Validation "Checksum mismatch for '$($modelAsset.name)'."
    }

    $stage = Join-Path $TmpDir "pyannote-install"
    if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
    if (Test-Path $runtimeDir) { Remove-Item -Recurse -Force $runtimeDir }
    New-Item -ItemType Directory -Force -Path $stage | Out-Null
    New-Item -ItemType Directory -Force -Path $runtimeDir | Out-Null
    Expand-Archive -Path $runtimeZip -DestinationPath $stage
    Expand-Archive -Path $modelZip -DestinationPath $stage

    $python = Join-Path $stage "python\python.exe"
    $modelDir = Join-Path $stage "model"
    if (-not (Test-Path -PathType Leaf $python)) {
        Fail-Validation "Pyannote runtime asset did not contain python/python.exe."
    }
    if (-not (Test-Path -PathType Container $modelDir)) {
        Fail-Validation "Pyannote model asset did not contain model/."
    }

    Move-Item (Join-Path $stage "python") (Join-Path $runtimeDir "python")
    Move-Item (Join-Path $stage "model") (Join-Path $runtimeDir "model")

    $now = [DateTime]::UtcNow.ToString("yyyy-MM-ddTHH:mm:ssZ")
    $installedManifest = [ordered]@{
        source         = "release_asset"
        app_version    = [string]$manifest.app_version
        compat_level   = [int]$manifest.compat_level
        runtime_asset  = [string]$runtimeAsset.name
        runtime_sha256 = [string]$runtimeAsset.sha256
        model_asset    = [string]$modelAsset.name
        model_sha256   = [string]$modelAsset.sha256
        runtime_arch   = "x86_64-pc-windows-msvc"
        installed_at   = $now
    }
    $status = [ordered]@{
        reason_code  = "ok"
        message      = "Pyannote diarization runtime is ready."
        updated_at   = $now
        validated_at = $now
    }
    ($installedManifest | ConvertTo-Json -Depth 6) | Set-Content -Encoding UTF8 (Join-Path $runtimeDir "manifest.json")
    ($status | ConvertTo-Json -Depth 6) | Set-Content -Encoding UTF8 (Join-Path $runtimeDir "status.json")
}

function Ensure-FixtureAudio {
    if ($FixtureAudio -and (Test-Path -PathType Leaf $FixtureAudio)) {
        return (Resolve-Path $FixtureAudio).Path
    }
    $dest = Join-Path $TmpDir "speech.wav"
    $url = "https://raw.githubusercontent.com/mudler/parakeet.cpp/9edf17c3ada66e0f881dcff155492867db7ac4cf/tests/fixtures/speech.wav"
    Invoke-WebRequest -Uri $url -OutFile $dest -UseBasicParsing
    if (-not (Test-Path -PathType Leaf $dest)) {
        Fail-Validation "Failed to download Windows validation fixture audio."
    }
    return $dest
}

function Invoke-TranscriptionSmoke {
    param([string]$AudioPath)
    $parakeet = Join-Path $DataDir "bin\parakeet-cli.exe"
    $modelPath = Join-Path $DataDir "parakeet-models\$ParakeetModel"
    $binDir = Join-Path $DataDir "bin"
    if (-not (Test-Path -PathType Leaf $parakeet)) {
        Fail-Validation "Managed Parakeet CLI was not installed at '$parakeet'."
    }
    if (-not (Test-Path -PathType Leaf $modelPath)) {
        Fail-Validation "Managed Parakeet model was not installed at '$modelPath'."
    }

    $previousPath = $env:PATH
    try {
        $env:PATH = "$binDir;$env:PATH"
        $env:PARAKEET_DEVICE = "cpu"
        $output = & $parakeet transcribe --model $modelPath --input $AudioPath --json 2>&1 | Out-String
        if ($LASTEXITCODE -ne 0) {
            Fail-Validation "Windows Parakeet transcription smoke failed: $output"
        }
        if ($output -notmatch '"text"\s*:' -and $output.Trim().Length -lt 8) {
            Fail-Validation "Windows Parakeet transcription smoke produced empty/malformed output: $output"
        }
    }
    finally {
        $env:PATH = $previousPath
        Remove-Item Env:PARAKEET_DEVICE -ErrorAction SilentlyContinue
    }
}

function Invoke-DiarizationSmoke {
    param([string]$AudioPath)
    $python = Join-Path $DataDir "runtime\pyannote\python\python.exe"
    $modelDir = Join-Path $DataDir "runtime\pyannote\model"
    $scriptPath = Join-Path $ScriptRoot "pyannote_diarize.py"
    $outputPath = Join-Path $TmpDir "pyannote-smoke.json"
    if (-not (Test-Path -PathType Leaf $python)) {
        Fail-Validation "Managed pyannote python was not installed at '$python'."
    }
    if (-not (Test-Path -PathType Container $modelDir)) {
        Fail-Validation "Managed pyannote model dir was not installed at '$modelDir'."
    }
    if (-not (Test-Path -PathType Leaf $scriptPath)) {
        Fail-Validation "Missing pyannote_diarize.py at '$scriptPath'."
    }

    $previousPath = $env:PATH
    $previousHome = $env:PYTHONHOME
    $previousPythonPath = $env:PYTHONPATH
    $pythonRoot = Split-Path -Parent $python
    try {
        $env:PYTHONHOME = $pythonRoot
        $env:PYTHONPATH = "$pythonRoot\Lib;$pythonRoot\Lib\site-packages"
        $env:PATH = "$pythonRoot;$pythonRoot\DLLs;$(Join-Path $DataDir 'bin');$env:PATH"
        & $python $scriptPath --audio-path $AudioPath --model-path $modelDir --device cpu | Set-Content -Encoding UTF8 $outputPath
        if ($LASTEXITCODE -ne 0) {
            Fail-Validation "Windows pyannote smoke failed with exit code $LASTEXITCODE."
        }
        $payload = Get-Content -Raw $outputPath | ConvertFrom-Json
        $labels = @($payload.speakers | ForEach-Object { $_.speaker_label } | Where-Object { $_ })
        if ($labels.Count -lt 1) {
            Fail-Validation "Pyannote smoke test did not produce speaker labels."
        }
    }
    finally {
        $env:PATH = $previousPath
        $env:PYTHONHOME = $previousHome
        $env:PYTHONPATH = $previousPythonPath
    }
}

if (-not ("SbobinoWindowProbe" -as [type])) {
    Add-Type -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public sealed class SbobinoWindowInfo
{
    public long Handle { get; set; }
    public uint ProcessId { get; set; }
    public string Title { get; set; }
    public string ClassName { get; set; }
}

public static class SbobinoWindowProbe
{
    private delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);

    [DllImport("user32.dll")]
    private static extern bool EnumWindows(EnumWindowsProc callback, IntPtr lParam);

    [DllImport("user32.dll")]
    private static extern bool IsWindowVisible(IntPtr hWnd);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetWindowText(IntPtr hWnd, StringBuilder text, int maxCount);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetClassName(IntPtr hWnd, StringBuilder className, int maxCount);

    [DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);

    public static SbobinoWindowInfo[] VisibleWindows()
    {
        var windows = new List<SbobinoWindowInfo>();
        EnumWindows((window, _) =>
        {
            if (!IsWindowVisible(window)) return true;
            uint processId;
            GetWindowThreadProcessId(window, out processId);
            var title = new StringBuilder(512);
            var className = new StringBuilder(256);
            GetWindowText(window, title, title.Capacity);
            GetClassName(window, className, className.Capacity);
            windows.Add(new SbobinoWindowInfo
            {
                Handle = window.ToInt64(),
                ProcessId = processId,
                Title = title.ToString(),
                ClassName = className.ToString()
            });
            return true;
        }, IntPtr.Zero);
        return windows.ToArray();
    }
}
"@
}

function Get-WindowProcessName {
    param([uint32]$ProcessId)
    try {
        return (Get-Process -Id $ProcessId -ErrorAction Stop).ProcessName
    }
    catch {
        return "<exited>"
    }
}

function Invoke-GuiContractSmoke {
    $commonConfigPath = Join-Path $RepoRoot "apps\desktop\src-tauri\tauri.conf.json"
    $windowsConfigPath = Join-Path $RepoRoot "apps\desktop\src-tauri\tauri.windows.conf.json"
    if (-not (Test-Path -PathType Leaf $commonConfigPath) -or -not (Test-Path -PathType Leaf $windowsConfigPath)) {
        Fail-Validation "Missing Tauri window config files for opaque-main-window validation."
    }
    $commonConfig = Get-Content $commonConfigPath -Raw | ConvertFrom-Json
    $windowsConfig = Get-Content $windowsConfigPath -Raw | ConvertFrom-Json
    $commonMainWindow = @($commonConfig.app.windows)[0]
    $windowsMainWindows = @($windowsConfig.app.windows)
    $macTransparencyPreserved = $commonMainWindow.transparent -eq $true
    $mainWindowOpaque =
        $windowsMainWindows.Count -eq 1 -and
        $windowsMainWindows[0].label -eq "main" -and
        $windowsMainWindows[0].transparent -eq $false -and
        -not [string]::IsNullOrWhiteSpace([string]$windowsMainWindows[0].backgroundColor)
    if (-not $mainWindowOpaque) {
        Fail-Validation "Windows main window is not configured as opaque."
    }
    if (-not $macTransparencyPreserved) {
        Fail-Validation "common/macOS transparency contract changed."
    }
    $ScenarioResults.opaque_main_window = "passed"

    $appProcesses = @(Get-Process -Name "sbobino-desktop" -ErrorAction SilentlyContinue)
    if ($appProcesses.Count -lt 1) {
        Fail-Validation "Sbobino process not running during GUI contract smoke."
    }
    $appPids = @($appProcesses | ForEach-Object { [uint32]$_.Id })

    $baselineHandles = [System.Collections.Generic.HashSet[long]]::new()
    foreach ($window in [SbobinoWindowProbe]::VisibleWindows()) {
        if ($appPids -notcontains $window.ProcessId) {
            [void]$baselineHandles.Add($window.Handle)
        }
    }

    $maxMainWindowCount = 0
    $suspicious = New-Object System.Collections.Generic.List[object]
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    while ([DateTime]::UtcNow -lt $deadline) {
        $visible = @([SbobinoWindowProbe]::VisibleWindows())
        $mainWindows = @($visible | Where-Object { ($appPids -contains $_.ProcessId) -and ($_.Title -eq "Sbobino") })
        $maxMainWindowCount = [Math]::Max($maxMainWindowCount, $mainWindows.Count)
        foreach ($window in $visible) {
            if ($appPids -contains $window.ProcessId) { continue }
            if ($baselineHandles.Contains($window.Handle)) { continue }
            $processName = Get-WindowProcessName -ProcessId $window.ProcessId
            if (
                $window.ClassName -match '(?i)ConsoleWindowClass|CASCADIA|PseudoConsole' -or
                $processName -match '(?i)^(conhost|OpenConsole|WindowsTerminal|cmd|powershell|pwsh|ffmpeg|whisper.*|parakeet.*|pythonw?)$'
            ) {
                $suspicious.Add([ordered]@{
                    handle = $window.Handle
                    process_id = $window.ProcessId
                    process_name = $processName
                    class_name = $window.ClassName
                    title = $window.Title
                }) | Out-Null
            }
        }
        Start-Sleep -Milliseconds 200
    }

    if ($maxMainWindowCount -lt 1) {
        Fail-Validation "Expected at least one visible Sbobino main window during GUI smoke."
    }
    if ($suspicious.Count -gt 0) {
        Fail-Validation ("Observed visible background console window(s): " + ($suspicious | ConvertTo-Json -Compress -Depth 5))
    }
    $ScenarioResults.no_visible_console_windows = "passed"
}

try {
    Write-Host "WINDOWS-PRIMARY clean-room validation for $Tag"

    Clear-InstallState

    $installerName = "Sbobino_${Version}_windows_x86_64-setup.exe"
    $installerPath = Join-Path $TmpDir $installerName
    Download-ReleaseAsset -AssetName $installerName -Destination $installerPath

    $install = Start-Process -FilePath $installerPath -ArgumentList "/S" -Wait -PassThru
    if ($install.ExitCode -ne 0) {
        Fail-Validation "NSIS installer exited with $($install.ExitCode)."
    }
    if (-not (Test-Path -PathType Leaf $AppPath)) {
        $match = Get-ChildItem $env:LOCALAPPDATA -Recurse -File -Filter "sbobino-desktop.exe" -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($null -eq $match) {
            Fail-Validation "Installed sbobino-desktop.exe was not found under LOCALAPPDATA."
        }
        $AppPath = $match.FullName
    }

    Seed-PrivacyAcceptance
    Launch-App
    Wait-ForSetupReportSuccess -Timeout $TimeoutSeconds
    Wait-ForManagedRuntimeReady -Timeout $TimeoutSeconds -RequirePyannote $false
    $ScenarioResults.clean_room_install = "passed"
    $ScenarioResults.first_setup = "passed"

    $audio = Ensure-FixtureAudio
    Invoke-TranscriptionSmoke -AudioPath $audio
    $ScenarioResults.functional_transcription_smoke = "passed"

    Stop-SbobinoProcesses
    Install-PyannoteReleaseAssets
    Set-SpeakerDiarizationEnabled -Enabled $true
    Launch-App
    Wait-ForManagedRuntimeReady -Timeout 900 -RequirePyannote $true
    $ScenarioResults.warm_restart = "passed"

    Invoke-DiarizationSmoke -AudioPath $audio
    $ScenarioResults.functional_diarization_smoke = "passed"

    Invoke-GuiContractSmoke

    $FinalStatus = "passed"
    $ReportNotes = "Windows runner completed clean-room install, first setup, transcription/diarization smokes, warm restart, and GUI contract checks."
    Write-ValidationReport
    Write-Host "WINDOWS-PRIMARY validation passed for $Tag"
}
catch {
    if ($FinalStatus -ne "failed" -or -not $ReportNotes) {
        $FinalStatus = "failed"
        $ReportNotes = [string]$_.Exception.Message
        Write-ValidationReport
    }
    throw
}
finally {
    Stop-SbobinoProcesses
    if (Test-Path $TmpDir) {
        Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $TmpDir
    }
}
