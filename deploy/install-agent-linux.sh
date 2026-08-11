#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  sudo ./install-agent-linux.sh \
    --binary ./pinglake-agent \
    --hub-url https://monitor.example.com \
    [--enrollment-token-file /secure/path/token] \
    [--sha256 SHA256] \
    [--allow-insecure-http] \
    [--name NODE_NAME]

The binary may also be an HTTPS URL. Remote URLs require --sha256.
Use --allow-insecure-http only for an explicitly trusted, isolated network.
If no token file or PINGLAKE_ENROLLMENT_TOKEN environment variable is set,
the installer prompts without echoing the token.
EOF
}

fail() {
  echo "$*" >&2
  exit 2
}

is_sha256() {
  [[ "$1" =~ ^[A-Fa-f0-9]{64}$ ]]
}

toml_escape() {
  local value="$1"
  if [[ "$value" == *$'\n'* || "$value" == *$'\r'* ]]; then
    fail "Configuration values must not contain line breaks."
  fi
  value="${value//\\/\\\\}"
  value="${value//\"/\\\"}"
  printf '%s' "$value"
}

BINARY_SOURCE=""
HUB_URL=""
ENROLLMENT_TOKEN="${PINGLAKE_ENROLLMENT_TOKEN:-}"
ENROLLMENT_TOKEN_FILE=""
EXPECTED_SHA256=""
ALLOW_INSECURE_HTTP=false
NODE_NAME="$(hostname)"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --binary) BINARY_SOURCE="${2:-}"; shift 2 ;;
    --hub-url) HUB_URL="${2:-}"; shift 2 ;;
    --enrollment-token-file) ENROLLMENT_TOKEN_FILE="${2:-}"; shift 2 ;;
    --sha256) EXPECTED_SHA256="${2:-}"; shift 2 ;;
    --allow-insecure-http) ALLOW_INSECURE_HTTP=true; shift ;;
    --name) NODE_NAME="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) fail "Unknown argument: $1" ;;
  esac
done

if [[ $EUID -ne 0 ]]; then
  echo "Run this installer as root." >&2
  exit 1
fi

if [[ -z "$BINARY_SOURCE" || -z "$HUB_URL" ]]; then
  usage
  exit 2
fi

if [[ "$HUB_URL" != https://* ]]; then
  if [[ "$HUB_URL" != http://* || "$ALLOW_INSECURE_HTTP" != true ]]; then
    fail "Hub URL must use HTTPS unless --allow-insecure-http is explicitly supplied."
  fi
fi

if [[ -n "$EXPECTED_SHA256" ]] && ! is_sha256 "$EXPECTED_SHA256"; then
  fail "--sha256 must be exactly 64 hexadecimal characters."
fi

if [[ -n "$ENROLLMENT_TOKEN_FILE" ]]; then
  if [[ ! -f "$ENROLLMENT_TOKEN_FILE" ]]; then
    fail "Enrollment token file is not a regular file."
  fi
  IFS= read -r ENROLLMENT_TOKEN <"$ENROLLMENT_TOKEN_FILE"
elif [[ -z "$ENROLLMENT_TOKEN" ]]; then
  read -r -s -p "Enrollment token: " ENROLLMENT_TOKEN
  echo
fi

if [[ -z "$ENROLLMENT_TOKEN" ]]; then
  fail "Enrollment token must not be empty."
fi

HUB_URL_ESCAPED="$(toml_escape "$HUB_URL")"
TOKEN_ESCAPED="$(toml_escape "$ENROLLMENT_TOKEN")"
NODE_NAME_ESCAPED="$(toml_escape "$NODE_NAME")"

if ! id pinglake >/dev/null 2>&1; then
  useradd --system --home-dir /var/lib/pinglake --shell /usr/sbin/nologin pinglake
fi

install -d -o root -g pinglake -m 0750 /etc/pinglake
install -d -o pinglake -g pinglake -m 0750 /var/lib/pinglake

staging_dir="$(mktemp -d /var/lib/pinglake/.install.XXXXXX)"
downloaded_binary="$staging_dir/pinglake-agent"
binary_path="/usr/local/bin/pinglake-agent"
config_path="/etc/pinglake/agent.toml"
service_path="/etc/systemd/system/pinglake-agent.service"
candidate_binary="$(mktemp /usr/local/bin/.pinglake-agent.XXXXXX)"
candidate_config="$(mktemp /etc/pinglake/.agent.toml.XXXXXX)"
candidate_service="$(mktemp /etc/systemd/system/.pinglake-agent.service.XXXXXX)"
binary_backup="$(mktemp /usr/local/bin/.pinglake-agent.backup.XXXXXX)"
config_backup="$(mktemp /etc/pinglake/.agent.toml.backup.XXXXXX)"
service_backup="$(mktemp /etc/systemd/system/.pinglake-agent.service.backup.XXXXXX)"
rm -f "$binary_backup" "$config_backup" "$service_backup"
had_binary=false
had_config=false
had_service=false
rollback_needed=false
was_active=false

cleanup() {
  local status=$?
  if [[ "$status" -ne 0 && "$rollback_needed" == true ]]; then
    echo "Installation failed; restoring the previous PingLake Agent files." >&2
    systemctl stop pinglake-agent.service >/dev/null 2>&1 || true
    if [[ "$had_binary" == true ]]; then
      mv -f "$binary_backup" "$binary_path" || true
    else
      rm -f "$binary_path"
    fi
    if [[ "$had_config" == true ]]; then
      mv -f "$config_backup" "$config_path" || true
    else
      rm -f "$config_path"
    fi
    if [[ "$had_service" == true ]]; then
      mv -f "$service_backup" "$service_path" || true
    else
      rm -f "$service_path"
      systemctl disable pinglake-agent.service >/dev/null 2>&1 || true
    fi
    systemctl daemon-reload >/dev/null 2>&1 || true
    if [[ "$was_active" == true ]]; then
      systemctl restart pinglake-agent.service >/dev/null 2>&1 || true
    fi
  fi
  rm -rf "$staging_dir"
  rm -f "$candidate_binary" "$candidate_config" "$candidate_service"
  rm -f "$binary_backup" "$config_backup" "$service_backup"
  exit "$status"
}
trap cleanup EXIT

if [[ "$BINARY_SOURCE" =~ ^https:// ]]; then
  if [[ -z "$EXPECTED_SHA256" ]]; then
    fail "Remote binary URLs require --sha256."
  fi
  curl --fail --location --proto '=https' --proto-redir '=https' --tlsv1.2 \
    --retry 3 --output "$downloaded_binary" "$BINARY_SOURCE"
else
  cp -- "$BINARY_SOURCE" "$downloaded_binary"
fi

if [[ -n "$EXPECTED_SHA256" ]]; then
  actual_sha256="$(sha256sum "$downloaded_binary" | awk '{print tolower($1)}')"
  if [[ "$actual_sha256" != "${EXPECTED_SHA256,,}" ]]; then
    fail "Binary SHA-256 does not match --sha256."
  fi
fi

install -o root -g root -m 0755 "$downloaded_binary" "$candidate_binary"
"$candidate_binary" --version >/dev/null

umask 0027
cat >"$candidate_config" <<EOF
hub_url = "${HUB_URL_ESCAPED}"
enrollment_token = "${TOKEN_ESCAPED}"
name = "${NODE_NAME_ESCAPED}"
interval_secs = 5
state_dir = "/var/lib/pinglake"
insecure_skip_verify = false
allow_insecure_http = ${ALLOW_INSECURE_HTTP}
EOF
chown pinglake:pinglake "$candidate_config"
chmod 0600 "$candidate_config"
install -o root -g root -m 0644 "$(dirname "$0")/pinglake-agent.service" "$candidate_service"

if systemctl is-active --quiet pinglake-agent.service; then
  was_active=true
fi

rollback_needed=true
if [[ "$was_active" == true ]]; then
  systemctl stop pinglake-agent.service
fi
if [[ -e "$binary_path" ]]; then
  cp -p -- "$binary_path" "$binary_backup"
  had_binary=true
fi
if [[ -e "$config_path" ]]; then
  cp -p -- "$config_path" "$config_backup"
  had_config=true
fi
if [[ -e "$service_path" ]]; then
  cp -p -- "$service_path" "$service_backup"
  had_service=true
fi

mv -f "$candidate_binary" "$binary_path"
mv -f "$candidate_config" "$config_path"
mv -f "$candidate_service" "$service_path"
systemctl daemon-reload
systemctl enable pinglake-agent.service
systemctl restart pinglake-agent.service
systemctl is-active --quiet pinglake-agent.service

rollback_needed=false
echo "PingLake Agent installed and running. Rotate the Hub enrollment token after all nodes enroll."
