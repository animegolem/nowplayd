#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "$0")" && pwd -P)"
repo_root="$(cd "$script_dir/.." && pwd -P)"
output="${1:-$repo_root/target/nowplayd.app}"

die() {
  echo "build-bundle.sh: ERROR: $*" >&2
  exit 2
}

[[ "$output" == /* && "$output" != "/" && "$output" == *.app ]] \
  || die "output must be an absolute .app path"
[[ -f "$script_dir/nowplayd.icns" ]] || die "missing packaging/nowplayd.icns"

cargo build --release --locked --manifest-path "$repo_root/Cargo.toml"

parent="$(dirname "$output")"
mkdir -p "$parent"
stage="$(mktemp -d "$parent/.nowplayd-bundle.XXXXXX")"
cleanup() {
  if [[ -n "${stage:-}" && -d "$stage" ]]; then
    rm -rf "$stage"
  fi
}
trap cleanup EXIT

mkdir -p "$stage/Contents/MacOS" "$stage/Contents/Resources"
cp "$repo_root/target/release/nowplayd" "$stage/Contents/MacOS/nowplayd"
cp "$script_dir/Info.plist" "$stage/Contents/Info.plist"
cp "$script_dir/nowplayd.icns" "$stage/Contents/Resources/nowplayd.icns"
chmod 755 "$stage/Contents/MacOS/nowplayd"
plutil -lint "$stage/Contents/Info.plist" >/dev/null
codesign --force --sign - --timestamp=none "$stage" >/dev/null

if [[ -e "$output" ]]; then
  rm -rf "$output"
fi
mv "$stage" "$output"
stage=""
echo "build-bundle.sh: built $output"
