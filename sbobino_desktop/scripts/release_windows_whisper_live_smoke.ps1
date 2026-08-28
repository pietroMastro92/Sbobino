param(
    [Parameter(Mandatory = $true)][string]$Version,
    [Parameter(Mandatory = $true)][string]$RepoSlug,
    [Parameter(Mandatory = $true)][string]$ReportPath,
    [string]$RuntimeZip = "",
    [int]$DurationSeconds = 900,
    [switch]$ExpectPreflightRejection
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
$tag = "v$Version"
$runDir = Join-Path $env:RUNNER_TEMP ("sbobino-whisper-live-" + [Guid]::NewGuid().ToString("N"))
$assetDir = Join-Path $runDir "assets"
$speechDir = Join-Path $runDir "speech"
$modelDir = Join-Path $runDir "models"
New-Item -ItemType Directory -Force $assetDir, $speechDir, $modelDir | Out-Null

try {
    if ($DurationSeconds -lt 65) { throw "Whisper live smoke duration must be at least 65 seconds" }
    if (-not $RuntimeZip) {
        gh release download $tag --repo $RepoSlug `
            --pattern "speech-runtime-windows-x86_64.zip" --dir $assetDir
        if ($LASTEXITCODE -ne 0) { throw "failed to download candidate speech runtime" }
        $RuntimeZip = Join-Path $assetDir "speech-runtime-windows-x86_64.zip"
    }
    if (-not (Test-Path -PathType Leaf $RuntimeZip)) { throw "speech runtime zip is missing: $RuntimeZip" }
    Expand-Archive -Path $RuntimeZip -DestinationPath $speechDir
    $speechRoot = Join-Path $speechDir "runtime"
    $whisper = Join-Path $speechRoot "bin\whisper-stream.exe"
    $ffmpeg = Join-Path $speechRoot "bin\ffmpeg.exe"
    if (-not (Test-Path -PathType Leaf $whisper)) { throw "packaged whisper-stream.exe is missing" }
    if (-not (Test-Path -PathType Leaf $ffmpeg)) { throw "packaged ffmpeg.exe is missing" }

    $binaryText = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($whisper))
    foreach ($marker in @("SBOBINO_WHISPER_REPLAY_WAV", "SBOBINO_WHISPER_LIVE_METRIC", "SBOBINO_WHISPER_LIVE_PREFLIGHT")) {
        if (-not $binaryText.Contains($marker)) { throw "packaged whisper-stream is missing $marker" }
    }

    $manifestPath = Join-Path $PSScriptRoot "..\crates\domain\src\whisper_live_model.json"
    if (-not (Test-Path -PathType Leaf $manifestPath)) {
        throw "Whisper live model manifest is missing: $manifestPath"
    }
    $manifest = Get-Content -Raw -Path $manifestPath | ConvertFrom-Json
    if ($manifest.schema_version -ne 1 -or
        $manifest.model -ne "tiny" -or
        $manifest.filename -ne "ggml-tiny-q8_0.bin" -or
        $manifest.url -match "/resolve/main/" -or
        $manifest.sha256 -notmatch "^[0-9a-fA-F]{64}$") {
        throw "Whisper live model manifest must pin the certified Tiny model with an immutable URL and SHA-256"
    }
    $modelSha = ([string]$manifest.sha256).ToLowerInvariant()
    $model = Join-Path $modelDir ([string]$manifest.filename)
    Invoke-WebRequest `
        -Uri ([string]$manifest.url) `
        -OutFile $model
    if ((Get-FileHash -Algorithm SHA256 $model).Hash.ToLowerInvariant() -ne $modelSha) {
        throw "pinned Whisper model checksum mismatch"
    }

    $fixtureSha = "5fceacff0315d49cb59fcc505bcecf1ed5f2f35c2897b1e65a59f30e5d922150"
    $fixture = Join-Path $runDir "speech.wav"
    Invoke-WebRequest `
        -Uri "https://raw.githubusercontent.com/mudler/parakeet.cpp/9edf17c3ada66e0f881dcff155492867db7ac4cf/tests/fixtures/speech.wav" `
        -OutFile $fixture
    if ((Get-FileHash -Algorithm SHA256 $fixture).Hash.ToLowerInvariant() -ne $fixtureSha) {
        throw "pinned live fixture checksum mismatch"
    }

    $audio = Join-Path $runDir "live-${DurationSeconds}s.wav"
    & $ffmpeg -hide_banner -loglevel error -y -stream_loop -1 -i $fixture `
        -t $DurationSeconds -ar 16000 -ac 1 -c:a pcm_s16le $audio
    if ($LASTEXITCODE -ne 0) { throw "packaged ffmpeg failed to prepare live replay audio" }

    $rawReport = Join-Path $runDir "live-input.json"
    $evaluatedReport = Join-Path $runDir "live-evaluated.json"
    $env:PATH = "$(Join-Path $speechRoot 'bin');$env:PATH"
    $liveArguments = @(
        "scripts/run_whisper_live_replay.py",
        "--binary", $whisper, "--model", $model, "--audio", $audio,
        "--fixture", $fixture, "--report", $rawReport, "--run-dir", $runDir,
        "--device", "cpu", "--platform", "windows-x86_64"
    )
    if ($ExpectPreflightRejection) { $liveArguments += "--expect-preflight-rejection" }
    python @liveArguments
    $runStatus = $LASTEXITCODE
    $recoveryDir = Join-Path $runDir "backlog-recovery"
    $recoveryReport = Join-Path $runDir "backlog-recovery.json"
    New-Item -ItemType Directory -Force $recoveryDir | Out-Null
    $env:SBOBINO_WHISPER_TEST_INFERENCE_DELAY_MS = "5000"
    python scripts/run_whisper_live_replay.py `
        --binary $whisper --model $model --audio $audio --fixture $fixture `
        --report $recoveryReport --run-dir $recoveryDir --device cpu `
        --platform windows-x86_64 --expect-backlog-recovery
    $recoveryStatus = $LASTEXITCODE
    Remove-Item Env:SBOBINO_WHISPER_TEST_INFERENCE_DELAY_MS -ErrorAction SilentlyContinue
    $raw = Get-Content $rawReport -Raw | ConvertFrom-Json
    $recovery = Get-Content $recoveryReport -Raw | ConvertFrom-Json
    if ($ExpectPreflightRejection) {
        $evaluateStatus = 0
        $evaluated = [PSCustomObject]@{
            schema_version = 1
            status = $raw.status
            engine = $raw.engine
            platform = $raw.platform
            metrics = [PSCustomObject]@{
                dropped_samples = $raw.dropped_samples
                missing_segments = $raw.missing_segments
                duplicate_segments = $raw.duplicate_segments
            }
            failures = @($raw.failures)
            live_mode = "preflight-rejected-incompatible-cpu"
            realtime_capable = $false
            preflight_rejected = $raw.preflight_rejected
            preflight = $raw.preflight
            requested_duration_seconds = $raw.requested_duration_seconds
            captured_duration_seconds = $raw.captured_duration_seconds
        }
    }
    else {
        python scripts/evaluate_live_latency.py $rawReport --report $evaluatedReport `
            --max-latency-seconds 2.0 --max-rss-growth-mib 256.0
        $evaluateStatus = $LASTEXITCODE
        $evaluated = Get-Content $evaluatedReport -Raw | ConvertFrom-Json
        $evaluated | Add-Member -Force live_mode "realtime"
        $evaluated | Add-Member -Force realtime_capable $true
        $evaluated | Add-Member -Force preflight_rejected $false
        $evaluated | Add-Member -Force preflight $raw.preflight
        $evaluated | Add-Member -Force requested_duration_seconds $raw.requested_duration_seconds
        $evaluated | Add-Member -Force captured_duration_seconds $raw.captured_duration_seconds
    }
    $evaluated.metrics | Add-Member -Force dropped_samples $raw.dropped_samples
    $evaluated.metrics | Add-Member -Force missing_segments $raw.missing_segments
    $evaluated.metrics | Add-Member -Force duplicate_segments $raw.duplicate_segments
    if ($raw.failures.Count -gt 0) {
        $evaluated.failures = @($evaluated.failures) + @($raw.failures)
        $evaluated.status = "failed"
    }
    if ($recovery.failures.Count -gt 0 -or $recovery.status -ne "passed") {
        $evaluated.failures = @($evaluated.failures) + @($recovery.failures | ForEach-Object { "backlog recovery: $_" })
        $evaluated.status = "failed"
    }
    $evaluated | Add-Member -Force evidence_class "hosted-packaged-engine"
    $evaluated | Add-Member -Force version $Version
    $evaluated | Add-Member -Force release_tag $tag
    $evaluated | Add-Member -Force real_engine $true
    $evaluated | Add-Member -Force real_harness $true
    $evaluated | Add-Member -Force runner "github-hosted windows-2025"
    $evaluated | Add-Member -Force harness "release_windows_whisper_live_smoke.ps1@v1"
    $evaluated | Add-Member -Force compute_device "cpu"
    $evaluated | Add-Member -Force duration_seconds $DurationSeconds
    $evaluated | Add-Member -Force commit_sha (git rev-parse HEAD)
    $evaluated | Add-Member -Force repo_slug $RepoSlug
    $evaluated | Add-Member -Force input_audio_sha256 ((Get-FileHash -Algorithm SHA256 $audio).Hash.ToLowerInvariant())
    $runtimeHashes = [PSCustomObject]@{
        "whisper-stream" = (Get-FileHash -Algorithm SHA256 $whisper).Hash.ToLowerInvariant()
        "whisper_model" = $modelSha
    }
    $evaluated | Add-Member -Force runtime_artifact_sha256 $runtimeHashes
    $evaluated | Add-Member -Force backlog_recovery ([PSCustomObject]@{
        status = $recovery.status
        live_mode = $recovery.live_mode
        backlog_recovery_expected = $recovery.backlog_recovery_expected
        preflight_rejection_expected = $recovery.preflight_rejection_expected
        preflight_rejected = $recovery.preflight_rejected
        captured_audio_frames = $recovery.captured_audio_frames
        saved_audio_frames = $recovery.saved_audio_frames
        dropped_samples = $recovery.dropped_samples
        backlog_reaction_seconds = $recovery.backlog_reaction_seconds
        backlog_reaction_budget_seconds = $recovery.backlog_reaction_budget_seconds
    })
    $parent = Split-Path -Parent $ReportPath
    if ($parent) { New-Item -ItemType Directory -Force $parent | Out-Null }
    $evaluated | ConvertTo-Json -Depth 8 | Set-Content -Encoding utf8 $ReportPath

    if ($runStatus -ne 0 -or $recoveryStatus -ne 0 -or $evaluateStatus -ne 0 -or $evaluated.status -ne "passed") {
        throw "packaged Windows Whisper live smoke failed; report written to $ReportPath"
    }
}
finally {
    if (Test-Path $runDir) { Remove-Item -Recurse -Force $runDir }
}
