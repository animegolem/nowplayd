---
node_id: AI-IMP-001
tags:
  - IMP-LIST
  - Implementation
  - spike
  - macos
kanban_status: planned
depends_on:
parent_epic: [[AI-EPIC-001-mpd-macos-now-playing-bridge]]
confidence_score: 0.6
date_created: 2026-08-16
date_completed:
---

# AI-IMP-001-macos-vertical-spike

## Summary of Issue #1

Platform risk is unretired: souvlaki 0.8.3 compiled and reached a
windowless winit loop in Sol's probe, but Control Center presentation,
command delivery, and true clearing are UNVERIFIED (SPEC.org §8). Per
the rev 0.3 ruling this DISPOSABLE spike runs first, on the intended
shipping path. Done-state: a human-verified pass of the §7-shaped
checks below from a bundled, launchd-launched, windowless binary, with
findings written back to SPEC.org §8 and this ticket; spike code is
then deletable without loss.

### Out of Scope

- Any MPD code. Metadata/artwork are static fixtures.
- Production shim design (AI-IMP-003) — this ticket only PROVES nil
  clearing and capability disabling are achievable from Rust.
- Reusable packaging scripts (AI-IMP-006).

### Design/Approach

Separate throwaway crate at `spike/` (excluded from the workspace
gates). Winit event loop with no window; souvlaki `MediaControls`
attached per its macOS requirements; static metadata + bundled JPEG
fixture (≥1000×1000, ≤1.5MB per §7) published on a timer-less
one-shot. Direct `objc2`/`msg_send` probes for: assigning nil to
`MPNowPlayingInfoCenter.nowPlayingInfo`, and disabling
`changePlaybackPositionCommand`. Command callbacks log to a file
(observable while headless). Hand-written `Info.plist` with
`LSUIElement`, ad-hoc bundle assembly by shell, loaded via a user
launchd agent.

### Files to Touch

- `spike/Cargo.toml`, `spike/src/main.rs`: the probe.
- `spike/bundle.sh`, `spike/Info.plist`, `spike/nowplayd-spike.plist`
  (launchd): ad-hoc packaging.
- `spike/fixture.jpg`: artwork fixture.
- `RAG/HUMAN-TESTING.md`: the live verification script.
- `SPEC.org` §8: findings folded (Review Lead applies).

### Implementation Checklist

<CRITICAL_RULE>
Before marking an item complete on the checklist MUST **stop** and
**think**. Have you validated all aspects are **implemented** and
**tested**?
</CRITICAL_RULE>

- [ ] `spike/` crate compiles standalone; NOT a workspace member.
- [ ] Windowless winit loop runs; no Dock icon, no window (LSUIElement
      bundle).
- [ ] souvlaki attach succeeds; static title/artist/album/duration/
      elapsed published.
- [ ] Fixture artwork published via file URL.
- [ ] toggle/play/pause/next/previous callbacks append to
      `/tmp/nowplayd-spike.log` with timestamps.
- [ ] Direct nil assignment to `nowPlayingInfo` verified to remove the
      Control Center entry (the §4 shim feasibility proof).
- [ ] `changePlaybackPositionCommand.enabled = false` verified: no
      scrubber presented.
- [ ] Bundle assembled by `spike/bundle.sh`; launches via
      `launchctl bootstrap gui/$UID` path.
- [ ] Clean termination clears Now Playing.
- [ ] Human-test script appended to `RAG/HUMAN-TESTING.md` incl.
      competing-player preconditions (Music.app playing before/after).
- [ ] Findings (what worked, what needed workarounds, exact objc2
      surface used) recorded in Issues Encountered for AI-IMP-003 to
      consume.

### Acceptance Criteria

**Scenario:** Spike bundle verified live by the owner.
**GIVEN** the assembled `.app` is loaded as a launchd user agent and
Music.app is playing.
**WHEN** the owner opens Control Center media controls.
**THEN** the spike's static track with artwork is presented (or the
documented arbitration behavior vs Music.app is recorded).
**WHEN** the owner presses each media key / control.
**THEN** each callback appears in the log within 1 s and no scrubber
is shown.
**WHEN** the agent is booted out.
**THEN** the Now Playing entry disappears entirely.

### Issues Encountered

<!--
The comments under the 'Issues Encountered' heading are the only
comments you MUST not remove
This section is filled out post work as you fill out the checklists.
You SHOULD document any issues encountered and resolved during the sprint.
You MUST document any failed implementations, blockers or missing tests.
-->
