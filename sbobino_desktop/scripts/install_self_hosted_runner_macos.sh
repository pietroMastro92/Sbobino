#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: install_self_hosted_runner_macos.sh <machine-class> [repo-slug] [runner-root]

Installs or refreshes a GitHub self-hosted runner for the Sbobino release matrix
on the current macOS machine and registers it with the required labels.

Supported machine classes:
  - AS-PRIMARY
  - AS-THIRD
  - INTEL-PRIMARY

Environment variables:
  GITHUB_RUNNER_VERSION          Override the actions runner version.
  SBOBINO_RUNNER_NAME           Override the generated runner name.
  SBOBINO_RUNNER_WORKDIR        Override the runner work directory.
  SBOBINO_RUNNER_EPHEMERAL      Set to 1 to configure the runner as ephemeral.
  SBOBINO_RUNNER_PATH           Override PATH exported by the LaunchAgent.
  SBOBINO_VALIDATION_FIXTURE_AUDIO
                                 Optional validation fixture path. AS-THIRD
                                 defaults to ~/Fixtures/as-third-diarization.wav.
EOF
}

if [[ $# -lt 1 || $# -gt 3 ]]; then
  usage
  exit 1
fi

MACHINE_CLASS=$1
REPO_SLUG=${2:-pietroMastro92/Sbobino}
MACHINE_CLASS_LOWER=$(printf '%s' "$MACHINE_CLASS" | tr '[:upper:]' '[:lower:]')
RUNNER_ROOT=${3:-"$HOME/.local/share/sbobino-gh-runners/${MACHINE_CLASS_LOWER}"}

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

need_cmd gh
need_cmd curl
need_cmd tar
need_cmd launchctl
need_cmd python3

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This installer only supports macOS." >&2
  exit 1
fi

labels_for_machine() {
  case "$1" in
    AS-PRIMARY)
      echo "self-hosted,macos,apple-silicon,as-primary"
      ;;
    AS-THIRD)
      echo "self-hosted,macos,apple-silicon,as-third"
      ;;
    INTEL-PRIMARY)
      echo "self-hosted,macos,x64,intel-primary"
      ;;
    *)
      usage
      exit 1
      ;;
  esac
}

expected_arch_for_machine() {
  case "$1" in
    AS-PRIMARY|AS-THIRD)
      echo "arm64"
      ;;
    INTEL-PRIMARY)
      echo "x86_64"
      ;;
    *)
      usage
      exit 1
      ;;
  esac
}

RUNNER_ARCH=$(uname -m)
EXPECTED_ARCH=$(expected_arch_for_machine "$MACHINE_CLASS")
if [[ "$RUNNER_ARCH" != "$EXPECTED_ARCH" ]]; then
  echo "Machine class $MACHINE_CLASS requires arch $EXPECTED_ARCH, but current host is $RUNNER_ARCH." >&2
  exit 1
fi

RUNNER_VERSION=${GITHUB_RUNNER_VERSION:-}
if [[ -z "${RUNNER_VERSION// }" ]]; then
  RUNNER_VERSION=$(gh api repos/actions/runner/releases/latest --jq '.tag_name' | sed 's/^v//')
fi

LABELS=$(labels_for_machine "$MACHINE_CLASS")
HOSTNAME_SAFE=$(scutil --get LocalHostName 2>/dev/null || hostname -s)
HOSTNAME_SAFE=${HOSTNAME_SAFE// /-}
RUNNER_NAME=${SBOBINO_RUNNER_NAME:-"sbobino-${MACHINE_CLASS_LOWER}-${HOSTNAME_SAFE}"}
RUNNER_WORKDIR=${SBOBINO_RUNNER_WORKDIR:-"$RUNNER_ROOT/_work"}
RUNNER_PATH=${SBOBINO_RUNNER_PATH:-"$HOME/.cargo/bin:/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"}
RUNNER_URL="https://github.com/$REPO_SLUG"
RUNNER_TARBALL="actions-runner-osx-${RUNNER_ARCH}-${RUNNER_VERSION}.tar.gz"
RUNNER_DOWNLOAD_URL="https://github.com/actions/runner/releases/download/v${RUNNER_VERSION}/${RUNNER_TARBALL}"
RUNNER_CONFIG_DIR="$RUNNER_ROOT/runner"
PLIST_PATH="$HOME/Library/LaunchAgents/com.sbobino.github-runner.${MACHINE_CLASS_LOWER}.plist"
TMP_DIR=$(mktemp -d)

cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

mkdir -p "$RUNNER_CONFIG_DIR" "$RUNNER_WORKDIR" "$(dirname "$PLIST_PATH")"

if [[ ! -x "$RUNNER_CONFIG_DIR/run.sh" ]]; then
  curl --fail --location --silent --show-error --output "$TMP_DIR/$RUNNER_TARBALL" "$RUNNER_DOWNLOAD_URL"
  tar -xzf "$TMP_DIR/$RUNNER_TARBALL" -C "$RUNNER_CONFIG_DIR"
fi

pushd "$RUNNER_CONFIG_DIR" >/dev/null

REGISTRATION_TOKEN=$(gh api \
  "repos/${REPO_SLUG}/actions/runners/registration-token" \
  -X POST \
  --jq '.token')

CONFIG_ARGS=(
  --unattended
  --url "$RUNNER_URL"
  --token "$REGISTRATION_TOKEN"
  --name "$RUNNER_NAME"
  --labels "$LABELS"
  --work "$RUNNER_WORKDIR"
  --replace
)

if [[ "${SBOBINO_RUNNER_EPHEMERAL:-0}" == "1" ]]; then
  CONFIG_ARGS+=(--ephemeral)
fi

./config.sh "${CONFIG_ARGS[@]}"

cat >"$PLIST_PATH" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>com.sbobino.github-runner.${MACHINE_CLASS_LOWER}</string>
  <key>ProgramArguments</key>
  <array>
    <string>$RUNNER_CONFIG_DIR/run.sh</string>
  </array>
  <key>WorkingDirectory</key>
  <string>$RUNNER_CONFIG_DIR</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>HOME</key>
    <string>$HOME</string>
    <key>PATH</key>
    <string>$RUNNER_PATH</string>
    <key>SBOBINO_VALIDATION_FIXTURE_AUDIO</key>
    <string>${SBOBINO_VALIDATION_FIXTURE_AUDIO:-$HOME/Fixtures/as-third-diarization.wav}</string>
  </dict>
  <key>KeepAlive</key>
  <true/>
  <key>RunAtLoad</key>
  <true/>
  <key>StandardOutPath</key>
  <string>$RUNNER_ROOT/runner.out.log</string>
  <key>StandardErrorPath</key>
  <string>$RUNNER_ROOT/runner.err.log</string>
</dict>
</plist>
EOF

launchctl bootout "gui/$(id -u)" "$PLIST_PATH" >/dev/null 2>&1 || true
launchctl bootstrap "gui/$(id -u)" "$PLIST_PATH"
launchctl enable "gui/$(id -u)/com.sbobino.github-runner.${MACHINE_CLASS_LOWER}"
launchctl kickstart -k "gui/$(id -u)/com.sbobino.github-runner.${MACHINE_CLASS_LOWER}"

popd >/dev/null

cat <<EOF
Self-hosted runner configured successfully.
  repo:         $REPO_SLUG
  machine:      $MACHINE_CLASS
  name:         $RUNNER_NAME
  labels:       $LABELS
  runner root:  $RUNNER_ROOT
  launch agent: $PLIST_PATH

Next recommended step:
  ./scripts/preflight_self_hosted_runner.sh "$MACHINE_CLASS" "$REPO_SLUG"

For AS-THIRD, make sure this fixture exists before running the VM gate:
  ~/Fixtures/as-third-diarization.wav
EOF
