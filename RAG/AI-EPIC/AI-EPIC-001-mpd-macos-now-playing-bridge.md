---
node_id: AI-EPIC-001
tags:
  - EPIC
  - AI
  - macos
  - mpd
  - media-keys
date_created: 2026-08-16
date_completed:
kanban_status: backlog
ai_imp_spawned: false
---

# AI-EPIC-001-mpd-macos-now-playing-bridge

## Problem Statement/Feature Scope

macOS media keys, Control Center, the lock screen, and AirPods gestures are
arbitrated by `mediaremoted`, which routes commands only to apps registered via
`MPNowPlayingInfoCenter`/`MPRemoteCommandCenter`. Terminal MPD frontends (rmpc,
ncmpcpp) structurally cannot register, so an MPD user on macOS has no hardware
media-key control and no Now Playing presence at all. Existing options are a
2012-era raw-event-tap tool (osxmpdkeys, pre-MediaRemote) or switching to a
native GUI client, which forfeits the terminal workflow. Because MPD holds all
playback state, one small peer daemon can close this gap for every frontend at
once.

## Proposed Solution(s)

Build `nowplayd`, a standalone Rust daemon that connects to MPD as an ordinary
client and mirrors it into the macOS media layer. From the user's point of
view: start the daemon once (launchd keeps it alive), and MPD behaves like a
first-class macOS media app — F7/F8/F9 and AirPods gestures control playback,
and Control Center / lock screen show the current track with album art —
regardless of which frontend (or none) is open.

Internally the daemon runs an MPD `idle` loop for change notifications, pulls
`status`/`currentsong`/`albumart` on change, and publishes via the `souvlaki`
crate, which wraps `MPRemoteCommandCenter` + `MPNowPlayingInfoCenter` on macOS
(and MPRIS on Linux as a free side effect). Remote command callbacks
(play/pause/next/prev) are translated back into MPD protocol commands. The
binary ships in a minimal `LSUIElement` app bundle with a launchd user agent.

Normative spec: `SPEC.org` (architecture, FRs, packaging).

## Path(s) Not Taken

- **rmpc frontend patch or rmpcd Lua plugin** — rmpcd's plugin API has full
  MPD access but no native-code ingress for media-key callbacks; upstream's
  own MPRIS is a native module, not a plugin. Owner ruled standalone daemon,
  matching upstream's daemon/frontend philosophy (rmpc FAQ).
- **Raw media-key event tap** (osxmpdkeys approach) — fights Music.app for
  keys, no Now Playing metadata, deprecated path.
- **Upstream PR adding a souvlaki module to rmpcd** — possible later; not a
  goal of this epic.

## Success Metrics

- With the daemon running and any frontend (or none), play/pause, next, and
  previous media keys control MPD with subjectively immediate response.
- Control Center and lock screen show correct title/artist/album, elapsed
  time, play state, and album art within 1 s of a song change.
- Daemon survives an MPD restart without user intervention (reconnects with
  backoff; Now Playing entry clears while disconnected).
- Installable from a clean clone via a documented one-command path (build +
  bundle + launchd agent); survives logout/login.
- `cargo build`, `cargo test`, and `cargo clippy -- -D warnings` all pass.

## Requirements

### Functional Requirements

- [ ] FR-1: The daemon shall connect to MPD at a configurable address
  (default `localhost:6600`, unix sockets supported) and track playback via
  `idle player mixer options`.
- [ ] FR-2: The daemon shall publish title, artist, album, duration, elapsed
  position, and play/pause state to macOS Now Playing on every relevant
  change.
- [ ] FR-3: The daemon shall register handlers for toggle/play/pause/next/
  previous remote commands and translate them into MPD commands.
- [ ] FR-4: The daemon shall fetch album art via `albumart`/`readpicture`
  and publish it as Now Playing artwork, skipping refetch when the song has
  not changed.
- [ ] FR-5: The daemon shall reconnect with backoff when MPD restarts or the
  connection drops, clearing the Now Playing entry while disconnected.
- [ ] FR-6: The project shall produce an `LSUIElement` app bundle and a
  launchd user agent plist, with an install path that is documented or
  scripted.
- [ ] FR-7: Configuration (MPD address, optional password) shall be readable
  from a config file and/or environment with a zero-config default.

### Non-Functional Requirements

- Idle CPU near zero: event-driven via `idle`; no polling loops.
- Single small binary; no runtime dependencies beyond MPD itself.
- Rust 2024 edition; gates: `cargo build` / `cargo test` /
  `cargo clippy -- -D warnings`.
- macOS-first, but no gratuitous platform lock-in: souvlaki's MPRIS backend
  should remain compiling (Linux CI-level validation acceptable).
- macOS media-layer behavior that cannot be validated headless is flagged
  per ticket for live manual verification, never claimed verified.
- Seek and volume commands are explicitly deferred; scope for them lives in
  SPEC.org per repo rules.

## Implementation Breakdown

_No AI-IMP tickets spawned yet. Proposed seams (pending owner/reviewer
sign-off): (1) MPD client + idle loop, (2) souvlaki integration incl.
NSRunLoop spike, (3) album art pipeline, (4) reconnect/lifecycle, (5) bundle +
launchd packaging. Mostly sequential; single-agent chain expected._
