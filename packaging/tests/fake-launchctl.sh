#!/usr/bin/env bash
set -euo pipefail

state="${NOWPLAYD_FAKE_LAUNCHCTL_STATE:?missing fake state path}"
log="${NOWPLAYD_FAKE_LAUNCHCTL_LOG:?missing fake log path}"

case "${1:-}" in
  print)
    [[ -f "$state" ]]
    ;;
  bootstrap)
    touch "$state"
    echo bootstrap >>"$log"
    ;;
  bootout)
    rm -f "$state"
    echo bootout >>"$log"
    ;;
  *)
    echo "fake-launchctl.sh: unexpected command ${1:-}" >&2
    exit 2
    ;;
esac
