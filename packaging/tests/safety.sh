#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../.." && pwd -P)"
temp_home="$(mktemp -d "${TMPDIR:-/tmp}/nowplayd-safety.XXXXXX")"
trap 'rm -rf "$temp_home"' EXIT

if HOME=/ "$repo_root/install.sh" --validate-only >/dev/null 2>&1; then
  echo "safety.sh: install accepted root HOME" >&2
  exit 1
fi
if HOME= "$repo_root/install.sh" --validate-only >/dev/null 2>&1; then
  echo "safety.sh: install accepted empty HOME" >&2
  exit 1
fi
if HOME=/ "$repo_root/uninstall.sh" --validate-only >/dev/null 2>&1; then
  echo "safety.sh: uninstall accepted root HOME" >&2
  exit 1
fi

HOME="$temp_home" "$repo_root/install.sh" --validate-only >/dev/null
HOME="$temp_home" "$repo_root/uninstall.sh" --validate-only >/dev/null
[[ ! -e "$temp_home/Applications/nowplayd.app" ]]
[[ ! -e "$temp_home/Library/LaunchAgents/io.github.animegolem.nowplayd.plist" ]]
echo "safety.sh: unsafe HOME rejected; exact temp targets accepted without mutation"
