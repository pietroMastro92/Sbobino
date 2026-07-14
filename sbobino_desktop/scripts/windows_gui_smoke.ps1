param(
    [Parameter(Mandatory = $true)]
    [string]$AppPath,
    [Parameter(Mandatory = $true)]
    [string]$HarnessPath,
    [Parameter(Mandatory = $true)]
    [string]$SpeechRuntimeZip,
    [Parameter(Mandatory = $true)]
    [string]$PyannoteRuntimeZip,
    [string]$ReportPath = "windows-gui-smoke-report.json",
    [int]$ObservationSeconds = 20,
    [int]$PollMilliseconds = 100
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path $AppPath -PathType Leaf)) {
    throw "Sbobino executable not found at '$AppPath'"
}
if (-not (Test-Path $HarnessPath -PathType Leaf)) {
    throw "background-process smoke harness not found at '$HarnessPath'"
}
if (-not (Test-Path $SpeechRuntimeZip -PathType Leaf)) {
    throw "speech runtime archive not found at '$SpeechRuntimeZip'"
}
if (-not (Test-Path $PyannoteRuntimeZip -PathType Leaf)) {
    throw "Pyannote runtime archive not found at '$PyannoteRuntimeZip'"
}
if ($ObservationSeconds -lt 5) {
    throw "ObservationSeconds must be at least 5"
}
if ($PollMilliseconds -lt 25) {
    throw "PollMilliseconds must be at least 25"
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$commonConfigPath = Join-Path $repoRoot "apps\desktop\src-tauri\tauri.conf.json"
$windowsConfigPath = Join-Path $repoRoot "apps\desktop\src-tauri\tauri.windows.conf.json"
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

$baselineHandles = [System.Collections.Generic.HashSet[long]]::new()
foreach ($window in [SbobinoWindowProbe]::VisibleWindows()) {
    [void]$baselineHandles.Add($window.Handle)
}

$observedWindows = @{}
$mainHandles = [System.Collections.Generic.HashSet[long]]::new()
$appProcess = $null
$harnessProcesses = @()
$extractRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("sbobino-gui-smoke-" + [guid]::NewGuid())

function Find-OneHelper {
    param([string]$Root, [string]$Name)
    $matches = @(Get-ChildItem $Root -Recurse -File -Filter $Name)
    if ($matches.Count -ne 1) {
        throw "expected exactly one $Name below '$Root', found $($matches.Count)"
    }
    return $matches[0].FullName
}

function Find-PyannotePython {
    param([string]$Root)
    $manifests = @(Get-ChildItem $Root -Recurse -File -Filter "sbobino-runtime.json")
    if ($manifests.Count -ne 1) {
        throw "expected exactly one Pyannote runtime manifest below '$Root', found $($manifests.Count)"
    }
    $python = Join-Path $manifests[0].Directory.FullName "python.exe"
    if (-not (Test-Path $python -PathType Leaf)) {
        throw "Pyannote runtime python.exe is missing beside '$($manifests[0].FullName)'"
    }
    return $python
}

try {
    $speechRoot = Join-Path $extractRoot "speech"
    $pyannoteRoot = Join-Path $extractRoot "pyannote"
    Expand-Archive -Path $SpeechRuntimeZip -DestinationPath $speechRoot
    Expand-Archive -Path $PyannoteRuntimeZip -DestinationPath $pyannoteRoot
    $helperProbes = @(
        @{ Name = "ffmpeg"; Path = (Find-OneHelper $speechRoot "ffmpeg.exe"); Args = @("-version") },
        @{ Name = "whisper"; Path = (Find-OneHelper $speechRoot "whisper-cli.exe"); Args = @("--help") },
        @{ Name = "parakeet"; Path = (Find-OneHelper $speechRoot "parakeet-cli.exe"); Args = @("--help") },
        @{ Name = "python"; Path = (Find-PyannotePython $pyannoteRoot); Args = @("--version") }
    )

    $appProcess = Start-Process -FilePath $AppPath -PassThru
    foreach ($probe in $helperProbes) {
        $harnessProcesses += Start-Process -FilePath $HarnessPath `
            -ArgumentList (@($probe.Path) + $probe.Args) `
            -PassThru
    }
    $deadline = [DateTime]::UtcNow.AddSeconds($ObservationSeconds)

    while ([DateTime]::UtcNow -lt $deadline) {
        foreach ($window in [SbobinoWindowProbe]::VisibleWindows()) {
            if ($window.ProcessId -eq $appProcess.Id) {
                [void]$mainHandles.Add($window.Handle)
            }
            if ($baselineHandles.Contains($window.Handle)) {
                continue
            }
            $key = [string]$window.Handle
            if (-not $observedWindows.ContainsKey($key)) {
                $observedWindows[$key] = [ordered]@{
                    handle = $window.Handle
                    process_id = $window.ProcessId
                    process_name = Get-WindowProcessName -ProcessId $window.ProcessId
                    class_name = $window.ClassName
                    title = $window.Title
                }
            }
        }
        Start-Sleep -Milliseconds $PollMilliseconds
    }

    $appProcess.Refresh()
    if ($appProcess.HasExited) {
        throw "Sbobino exited during the GUI smoke with code $($appProcess.ExitCode)"
    }
    foreach ($process in $harnessProcesses) {
        $process.WaitForExit()
        if ($process.ExitCode -ne 0) {
            throw "background-process smoke harness exited with $($process.ExitCode)"
        }
    }
}
finally {
    foreach ($process in $harnessProcesses) {
        if (-not $process.HasExited) {
            Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        }
    }
    if ($appProcess -and -not $appProcess.HasExited) {
        Stop-Process -Id $appProcess.Id -Force -ErrorAction SilentlyContinue
    }
    Remove-Item $extractRoot -Recurse -Force -ErrorAction SilentlyContinue
}

$suspiciousWindows = @($observedWindows.Values | Where-Object {
    $_.class_name -match '(?i)ConsoleWindowClass|CASCADIA|PseudoConsole' -or
    $_.process_name -match '(?i)^(conhost|OpenConsole|WindowsTerminal|cmd|powershell|pwsh|ffmpeg|whisper.*|parakeet.*|pythonw?|windows-background-process-smoke)$'
})

$report = [ordered]@{
    schema_version = 1
    platform = "windows"
    status = "passed"
    tested_at_utc = [DateTime]::UtcNow.ToString("yyyy-MM-ddTHH:mm:ssZ")
    observation_seconds = $ObservationSeconds
    poll_milliseconds = $PollMilliseconds
    visible_console_windows = $suspiciousWindows.Count
    main_window_count = $mainHandles.Count
    main_window_opaque = $mainWindowOpaque
    macos_transparency_preserved = $macTransparencyPreserved
    helper_probes = @($helperProbes | ForEach-Object { $_.Name })
    suspicious_windows = $suspiciousWindows
}

$failures = @()
if (-not $mainWindowOpaque) { $failures += "Windows main window is not configured as opaque" }
if (-not $macTransparencyPreserved) { $failures += "common/macOS transparency contract changed" }
if ($mainHandles.Count -ne 1) { $failures += "expected exactly one Sbobino main window, observed $($mainHandles.Count)" }
if ($suspiciousWindows.Count -ne 0) { $failures += "observed $($suspiciousWindows.Count) visible background console window(s)" }

if ($failures.Count -gt 0) {
    $report.status = "failed"
    $report.failures = $failures
}

$reportDirectory = Split-Path -Parent $ReportPath
if ($reportDirectory) {
    New-Item -ItemType Directory -Force $reportDirectory | Out-Null
}
$report | ConvertTo-Json -Depth 8 | Set-Content -Encoding UTF8 $ReportPath

if ($failures.Count -gt 0) {
    throw ($failures -join "; ")
}

Write-Host "Windows GUI smoke passed: one opaque Sbobino window, zero visible background consoles."
