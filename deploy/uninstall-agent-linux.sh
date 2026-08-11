#!/usr/bin/env bash
set -euo pipefail

if [[ $EUID -ne 0 ]]; then
  echo "Run this uninstaller as root." >&2
  exit 1
fi

systemctl disable --now pinglake-agent.service 2>/dev/null || true
rm -f /etc/systemd/system/pinglake-agent.service
systemctl daemon-reload
rm -f /usr/local/bin/pinglake-agent
rm -rf /etc/pinglake

echo "PingLake Agent removed. State remains in /var/lib/pinglake."
echo "Delete that directory manually only when you do not plan to reuse this node identity."
