---
node_id: AI-LOG-001
tags:
  - AI-log
  - development-summary
  - macos
  - spike
closed_tickets: []
date_created: 2026-08-16
related_files:
  - RAG/AI-IMP/AI-IMP-001-macos-vertical-spike.md
  - spike/src/main.rs
  - spike/bundle.sh
confidence_score: 0.95
---

# 2026-08-16-LOG-AI-windowless-now-playing-spike

## Work Completed

Implemented the disposable AI-IMP-001 crate, deterministic JPEG fixture,
`LSUIElement` app bundle, and launchd user agent. The spike publishes a static
full metadata fixture through souvlaki, registers the five required command
callbacks, disables the position command through a narrow native probe, and
performs generation-safe true clearing on SIGTERM/SIGINT.

The assembled release binary was bootstrapped through launchd. Its native
self-checks reported the scrubber disabled and `nowPlayingInfo` nil after a
real bootout. The uninstall path removed the process, launchd job, app bundle,
installed plist, and log. The formal owner-visible Control Center matrix is
still pending, so AI-IMP-001 remains open.

## Session Commits

- `imp/001` contains one atomic AI-IMP-001 implementation commit. The exact
  SHA is reported in the channel submission after commit creation.

## Issues Encountered

- The designated `code-lead/primary` clone had intentional uncommitted
  Code Lead-specific `CLAUDE.md` wiring that overlaps current `main`.
  Implementation used a fresh independent `code-lead/imp-001` clone so that
  state remained untouched.
- Redirecting stderr and explicitly appending to the same launchd log caused
  duplicate lines on the first run. Explicit file logging is now the primary
  path; stderr is only the failure fallback.
- The first owner pass exposed a harness-state issue: Pause was delivered, but
  the probe never published Paused, so macOS could not subsequently offer Play.
  Remote callbacks now return to the winit thread and update the published
  playback state. The concurrent-session screenshot also showed a multi-card
  picker rather than the assumed single exclusive owner.
- The amended owner pass delivered toggle/play/pause/next/previous with
  timestamps while competing sessions remained visible. Teardown exposed an
  asynchronous launchd-removal race, so the packaging script now waits a
  bounded five seconds for the job to disappear before removing artifacts.
- Computer Use could not address the Control Center process or synthesize
  macOS media keys. This is recorded as a human-gate limitation rather than a
  live verification claim.

## Tests Added

One pure transition test covers Pause → paused, Play → playing, both Toggle
directions, and the state-neutral Next/Previous events. Validation also covered
standalone build/test/clippy, both plist files, shell lint, a real launchd
bootstrap, native capability readback, SIGTERM cleanup, process/job absence,
and exact artifact removal.

## Next Steps

1. Review Lead appends the proposed split live matrix to
   `RAG/HUMAN-TESTING.md`.
2. Owner confirms the spike card is visually absent after final teardown.
3. Review Lead rules whether the stronger concurrent-session presentation and
   five-command pass supersede the original uncontested/single-owner premise,
   records results in SPEC.org §8, and only then opens AI-IMP-002.
