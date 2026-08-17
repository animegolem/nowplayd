#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../.." && pwd -P)"
rustup_home="${RUSTUP_HOME:-$HOME/.rustup}"
cargo_home="${CARGO_HOME:-$HOME/.cargo}"
test_root="$(mktemp -d "${TMPDIR:-/tmp}/nowplayd-idempotence.XXXXXX")"
trap 'rm -rf "$test_root"' EXIT
test_home="$test_root/home"
state="$test_root/loaded"
log="$test_root/launchctl.log"
mkdir -p "$test_home/.config/nowplayd"
printf 'mpd_password = "sentinel-test-only"\n' >"$test_home/.config/nowplayd/config.toml"
chmod 644 "$test_home/.config/nowplayd/config.toml"

run_install() {
  HOME="$test_home" \
    RUSTUP_HOME="$rustup_home" \
    CARGO_HOME="$cargo_home" \
    NOWPLAYD_LAUNCHCTL="$repo_root/packaging/tests/fake-launchctl.sh" \
    NOWPLAYD_FAKE_LAUNCHCTL_STATE="$state" \
    NOWPLAYD_FAKE_LAUNCHCTL_LOG="$log" \
    "$repo_root/install.sh"
}

run_uninstall() {
  HOME="$test_home" \
    NOWPLAYD_LAUNCHCTL="$repo_root/packaging/tests/fake-launchctl.sh" \
    NOWPLAYD_FAKE_LAUNCHCTL_STATE="$state" \
    NOWPLAYD_FAKE_LAUNCHCTL_LOG="$log" \
    "$repo_root/uninstall.sh"
}

run_install >"$test_root/first.out" 2>"$test_root/first.err"
[[ "$(stat -f '%Lp' "$test_home/.config/nowplayd/config.toml")" == "600" ]]
run_install >"$test_root/second.out" 2>"$test_root/second.err"
grep -q 'no changes' "$test_root/second.out"
[[ "$(grep -c '^bootstrap$' "$log")" == "1" ]]
if grep -q '^bootout$' "$log"; then
  echo "idempotence.sh: unchanged install booted out the agent" >&2
  exit 1
fi

/usr/libexec/PlistBuddy -c 'Set :CFBundleVersion 999' \
  "$test_home/Applications/nowplayd.app/Contents/Info.plist"
run_install >"$test_root/update.out" 2>"$test_root/update.err"
[[ "$(grep -c '^bootout$' "$log")" == "1" ]]
[[ "$(grep -c '^bootstrap$' "$log")" == "2" ]]
[[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$test_home/Applications/nowplayd.app/Contents/Info.plist")" == "1" ]]

mkdir -p "$test_home/Library/Caches/nowplayd"
run_uninstall >"$test_root/uninstall.out"
run_uninstall >"$test_root/uninstall-again.out"
grep -q 'nothing to remove' "$test_root/uninstall-again.out"
[[ -f "$test_home/.config/nowplayd/config.toml" ]]
[[ ! -e "$test_home/Applications/nowplayd.app" ]]
[[ ! -e "$test_home/Library/LaunchAgents/io.github.animegolem.nowplayd.plist" ]]
[[ ! -e "$test_home/Library/Caches/nowplayd" ]]
echo "idempotence.sh: no-op install, changed update, permissions, and double uninstall passed"
