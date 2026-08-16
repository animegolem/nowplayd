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
kanban_status: in-progress
ai_imp_spawned: true
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

Normative definitions live in SPEC.org §11 (rev 0.3); this epic is a
projection and tracks completion only. Cite `SPEC.org §11 FR-n` in tickets.

- [ ] FR-1: MPD connection + dedicated idle listener (§11 FR-1, §5.1).
- [ ] FR-2: Metadata/state publishing within the §7 bound (§11 FR-2).
- [ ] FR-3: Remote commands incl. disabled position capability (§11 FR-3).
- [ ] FR-4: Artwork via the atomic cache pipeline (§11 FR-4, §5.3).
- [ ] FR-5: True clearing + reconnect with backoff (§11 FR-5).
- [ ] FR-6: Bundle + launchd agent, idempotent install (§11 FR-6).
- [ ] FR-7: Config — zero-config default, TOML, env overrides (§11 FR-7).
- [ ] FR-8: Observability logging (§11 FR-8).
- [ ] FR-9: Idempotent install/update, documented uninstall (§11 FR-9).

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

Cut 2026-08-16 (owner go given; order ruled at SPEC.org rev 0.3, Sol
pre-ticket review — platform risk retires first). All await round-1
pre-implementation review; every confidence_score is below the 0.9
skip threshold except none, so round-1 is universal.

- AI-IMP-001 — bundled windowless macOS vertical spike (disposable). FR feasibility for FR-2/3/5 clearing/capability claims.
- AI-IMP-002 — pure state model + two-connection MPD transport (FR-1, data half of FR-2).
- AI-IMP-003 — platform adapter + §4 shim: native clear, scrubber disabled, commands → MPD (FR-2, FR-3). Depends: 001, 002.
- AI-IMP-004 — atomic artwork cache pipeline (FR-4). Depends: 002, 003.
- AI-IMP-005 — reconnect + shutdown lifecycle, true clearing policy (FR-5). Depends: 002, 003.
- AI-IMP-006 — config, logging, bundle, idempotent install/uninstall (FR-6..FR-9). Depends: 001, 003, 005.
