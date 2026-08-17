#!/usr/bin/env bash
set -euo pipefail

home="${HOME:-}"
label="io.github.animegolem.nowplayd"
domain="gui/$(id -u)"
app_target="$home/Applications/nowplayd.app"
plist_target="$home/Library/LaunchAgents/$label.plist"
cache_target="$home/Library/Caches/nowplayd"
config_target="$home/.config/nowplayd/config.toml"
launchctl_bin="${NOWPLAYD_LAUNCHCTL:-launchctl}"

die() {
  echo "uninstall.sh: ERROR: $*" >&2
  exit 2
}

validate_targets() {
  [[ -n "$home" && "$home" == /* && "$home" != "/" ]] || die "unsafe or empty HOME"
  [[ "$app_target" == "$home/Applications/nowplayd.app" ]] || die "unexpected app target"
  [[ "$plist_target" == "$home/Library/LaunchAgents/$label.plist" ]] || die "unexpected plist target"
  [[ "$cache_target" == "$home/Library/Caches/nowplayd" ]] || die "unexpected cache target"
}

is_loaded() {
  "$launchctl_bin" print "$domain/$label" >/dev/null 2>&1
}

wait_until_unloaded() {
  local remaining=50
  while is_loaded; do
    ((remaining > 0)) || return 1
    sleep 0.1
    remaining=$((remaining - 1))
  done
}

validate_targets
if [[ "${1:-}" == "--validate-only" ]]; then
  echo "uninstall.sh: targets validated"
  exit 0
fi
[[ $# -eq 0 ]] || die "usage: ./uninstall.sh [--validate-only]"

if is_loaded; then
  "$launchctl_bin" bootout "$domain/$label"
  wait_until_unloaded || die "$domain/$label remains loaded 5 seconds after bootout"
fi

removed=false
if [[ -d "$app_target" ]]; then
  rm -rf "$app_target"
  removed=true
fi
if [[ -f "$plist_target" ]]; then
  rm -f "$plist_target"
  removed=true
fi
if [[ -d "$cache_target" ]]; then
  rm -rf "$cache_target"
  removed=true
fi

if $removed; then
  echo "uninstall.sh: removed agent, bundle, plist, and artwork cache"
else
  echo "uninstall.sh: nothing to remove"
fi
if [[ -f "$config_target" ]]; then
  echo "uninstall.sh: preserved config $config_target"
else
  echo "uninstall.sh: config path remains available at $config_target"
fi
