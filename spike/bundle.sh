#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "$0")" && pwd -P)"
app_root="$HOME/Applications/Nowplayd Spike.app"
agent_plist="$HOME/Library/LaunchAgents/org.nowplayd.spike.plist"
label="org.nowplayd.spike"
domain="gui/$UID"
log_path="/tmp/nowplayd-spike.log"

die() {
  echo "bundle.sh: ERROR: $*" >&2
  exit 2
}

validate_targets() {
  [[ -n "$HOME" && "$HOME" != "/" ]] || die "unsafe HOME"
  [[ "$app_root" == "$HOME/Applications/Nowplayd Spike.app" ]] || die "unexpected app target"
  [[ "$agent_plist" == "$HOME/Library/LaunchAgents/org.nowplayd.spike.plist" ]] || die "unexpected plist target"
  [[ "$log_path" == "/tmp/nowplayd-spike.log" ]] || die "unexpected log target"
}

is_loaded() {
  launchctl print "$domain/$label" >/dev/null 2>&1
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
    launchctl bootout "$domain/$label"
    wait_until_unloaded || die "$domain/$label remains loaded 5 s after bootout"
  fi
}

build_bundle() {
  cargo build --release --manifest-path "$script_dir/Cargo.toml"

  local stage
  stage="$(mktemp -d "${TMPDIR:-/tmp}/nowplayd-spike.XXXXXX")"
  trap 'rm -rf "$stage"' RETURN

  mkdir -p "$stage/Contents/MacOS" "$stage/Contents/Resources"
  cp "$script_dir/target/release/nowplayd-spike" "$stage/Contents/MacOS/nowplayd-spike"
  cp "$script_dir/Info.plist" "$stage/Contents/Info.plist"
  cp "$script_dir/fixture.jpg" "$stage/Contents/Resources/fixture.jpg"
  chmod 755 "$stage/Contents/MacOS/nowplayd-spike"

  mkdir -p "$(dirname "$app_root")"
  rm -rf "$app_root"
  mv "$stage" "$app_root"
  trap - RETURN
}

write_agent_plist() {
  mkdir -p "$(dirname "$agent_plist")"
  cp "$script_dir/nowplayd-spike.plist" "$agent_plist"
  /usr/libexec/PlistBuddy -c \
    "Set :ProgramArguments:0 $app_root/Contents/MacOS/nowplayd-spike" \
    "$agent_plist"
  plutil -lint "$agent_plist"
}

install() {
  validate_targets
  bootout_if_loaded
  build_bundle
  write_agent_plist
  : >"$log_path"
  launchctl bootstrap "$domain" "$agent_plist"
  launchctl print "$domain/$label" >/dev/null
  echo "bundle.sh: installed and loaded $domain/$label"
  echo "bundle.sh: log $log_path"
}

uninstall() {
  validate_targets
  bootout_if_loaded
  rm -rf "$app_root"
  rm -f "$agent_plist" "$log_path"
  if is_loaded; then
    die "$domain/$label remains loaded after bootout"
  fi
  echo "bundle.sh: removed agent, bundle, plist, and log"
}

status() {
  if is_loaded; then
    launchctl print "$domain/$label"
  else
    echo "bundle.sh: $domain/$label is not loaded"
    return 1
  fi
}

case "${1:-}" in
  install) install ;;
  uninstall) uninstall ;;
  status) status ;;
  *) die "usage: $0 {install|uninstall|status}" ;;
esac
