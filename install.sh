#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")" && pwd -P)"
home="${HOME:-}"
label="io.github.animegolem.nowplayd"
domain="gui/$(id -u)"
app_target="$home/Applications/nowplayd.app"
plist_target="$home/Library/LaunchAgents/$label.plist"
cache_target="$home/Library/Caches/nowplayd"
log_target="$home/Library/Logs/nowplayd.log"
config_target="$home/.config/nowplayd/config.toml"
launchctl_bin="${NOWPLAYD_LAUNCHCTL:-launchctl}"

die() {
  echo "install.sh: ERROR: $*" >&2
  exit 2
}

validate_targets() {
  [[ -n "$home" && "$home" == /* && "$home" != "/" ]] || die "unsafe or empty HOME"
  [[ "$app_target" == "$home/Applications/nowplayd.app" ]] || die "unexpected app target"
  [[ "$plist_target" == "$home/Library/LaunchAgents/$label.plist" ]] || die "unexpected plist target"
  [[ "$cache_target" == "$home/Library/Caches/nowplayd" ]] || die "unexpected cache target"
  [[ "$log_target" == "$home/Library/Logs/nowplayd.log" ]] || die "unexpected log target"
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

bootout_if_loaded() {
  if is_loaded; then
    "$launchctl_bin" bootout "$domain/$label"
    wait_until_unloaded || die "$domain/$label remains loaded 5 seconds after bootout"
  fi
}

enforce_config_permissions() {
  if [[ -f "$config_target" ]] && grep -Eq '^[[:space:]]*mpd_password[[:space:]]*=' "$config_target"; then
    chmod 600 "$config_target"
    [[ "$(stat -f '%Lp' "$config_target")" == "600" ]] \
      || die "could not enforce 0600 on $config_target"
  fi
}

json_quote() {
  local value="$1"
  value="${value//\\/\\\\}"
  value="${value//\"/\\\"}"
  value="${value//$'\n'/\\n}"
  value="${value//$'\r'/\\r}"
  value="${value//$'\t'/\\t}"
  printf '"%s"' "$value"
}

validate_targets
if [[ "${1:-}" == "--validate-only" ]]; then
  echo "install.sh: targets validated"
  exit 0
fi
[[ $# -eq 0 ]] || die "usage: ./install.sh [--validate-only]"

stage_root="$(mktemp -d "${TMPDIR:-/tmp}/nowplayd-install.XXXXXX")"
cleanup() {
  if [[ -n "${stage_root:-}" && -d "$stage_root" ]]; then
    rm -rf "$stage_root"
  fi
}
trap cleanup EXIT

stage_app="$stage_root/nowplayd.app"
stage_plist="$stage_root/$label.plist"
binary_target="$app_target/Contents/MacOS/nowplayd"
program_arguments_json="[$(json_quote "$binary_target")]"
"$repo_root/packaging/build-bundle.sh" "$stage_app"
cp "$repo_root/packaging/$label.plist.tmpl" "$stage_plist"
plutil -replace ProgramArguments -json "$program_arguments_json" "$stage_plist"
plutil -replace StandardErrorPath -string "$log_target" "$stage_plist"
plutil -lint "$stage_plist" >/dev/null
program_argument_count="$(plutil -extract ProgramArguments raw -o - "$stage_plist")"
program_argument="$(plutil -extract ProgramArguments.0 raw -o - "$stage_plist")"
[[ "$program_argument_count" == "1" && "$program_argument" == "$binary_target" ]] \
  || die "staged plist ProgramArguments does not exactly match the app binary"

enforce_config_permissions
"$stage_app/Contents/MacOS/nowplayd" --check-config

app_changed=true
plist_changed=true
if [[ -d "$app_target" ]] && diff -qr "$stage_app" "$app_target" >/dev/null; then
  app_changed=false
fi
if [[ -f "$plist_target" ]] && cmp -s "$stage_plist" "$plist_target"; then
  plist_changed=false
fi

if ! $app_changed && ! $plist_changed && is_loaded; then
  echo "install.sh: no changes; $domain/$label is already loaded"
  exit 0
fi

if $app_changed || $plist_changed; then
  bootout_if_loaded
fi

mkdir -p "$(dirname "$app_target")" "$(dirname "$plist_target")" "$(dirname "$log_target")"
if $app_changed; then
  previous="$stage_root/previous.app"
  if [[ -e "$app_target" ]]; then
    mv "$app_target" "$previous"
  fi
  if ! mv "$stage_app" "$app_target"; then
    [[ -d "$previous" ]] && mv "$previous" "$app_target"
    die "staged app replacement failed"
  fi
  [[ -d "$previous" ]] && rm -rf "$previous"
fi
if $plist_changed; then
  plist_stage="$plist_target.tmp.$$"
  cp "$stage_plist" "$plist_stage"
  chmod 600 "$plist_stage"
  mv "$plist_stage" "$plist_target"
fi
touch "$log_target"

if ! is_loaded; then
  "$launchctl_bin" bootstrap "$domain" "$plist_target"
fi
"$launchctl_bin" print "$domain/$label" >/dev/null
echo "install.sh: installed and loaded $domain/$label"
echo "install.sh: log $log_target (v1 is unrotated)"
