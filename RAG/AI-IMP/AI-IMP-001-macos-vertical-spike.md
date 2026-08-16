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

- [x] `spike/` crate compiles standalone; NOT a workspace member.
- [x] Windowless winit loop runs; no Dock icon, no window (LSUIElement
      bundle).
- [x] souvlaki attach succeeds; static title/artist/album/duration/
      elapsed published.
- [x] Fixture artwork published via file URL.
- [x] toggle/play/pause/next/previous callbacks append to
      `/tmp/nowplayd-spike.log` with timestamps.
- [x] Direct nil assignment to `nowPlayingInfo` verified to remove the
      Control Center entry (the §4 shim feasibility proof).
- [x] `changePlaybackPositionCommand.enabled = false` verified: no
      scrubber presented.
- [x] Bundle assembled by `spike/bundle.sh`; launches via
      `launchctl bootstrap gui/$UID` path.
- [ ] Clean termination clears Now Playing.
- [ ] Human-test script appended to `RAG/HUMAN-TESTING.md` with the
      rev 0.4 split matrix: UNCONTESTED BASELINE (mandatory pass, no
      competing Now Playing session) and ARBITRATION
      CHARACTERIZATION (observational: Music.app stopped/paused/
      playing before and after activation; record ownership and
      regain transitions).
- [x] Spike seeded from Sol's probe (souvlaki 0.8.3 + winit 0.30.13,
      `.inbox` attachment) rather than rediscovered.
- [x] Teardown path: `launchctl bootout` the spike job and remove
      its bundle, plist, and log — the disposable experiment leaves
      NO live agent behind (verified by re-listing launchd jobs).
- [x] Findings (what worked, what needed workarounds, exact objc2
      surface used) recorded in Issues Encountered for AI-IMP-003 to
      consume; live result handed to Review Lead to fold into SPEC.org
      §8 — a passing baseline ruling is the gate that authorizes
      IMP-002 onward (§6).

### Acceptance Criteria

**Scenario 1 (uncontested baseline — MANDATORY PASS; failure fails
the ticket and blocks IMP-002+ per §6):**
**GIVEN** the assembled `.app` is loaded as a launchd user agent and
NO other app holds an active Now Playing session.
**WHEN** the owner opens Control Center media controls.
**THEN** the spike's static track with artwork is presented.
**WHEN** the owner exercises all five commands
(toggle/play/pause/next/previous).
**THEN** each callback appears in the log within 1 s and no scrubber
is shown.
**WHEN** the agent is booted out.
**THEN** the Now Playing entry disappears entirely and teardown
leaves no agent, bundle, plist, or log behind.

**Scenario 2 (arbitration characterization — OBSERVATIONAL, cannot
fail the ticket):**
**GIVEN** Music.app stopped / paused / playing, before and after
spike activation (each combination).
**WHEN** the owner inspects system media controls.
**THEN** which source owns them, and any transition needed for the
spike to regain ownership, is recorded verbatim into Issues
Encountered for the §8 fold.

### Issues Encountered

<!--
The comments under the 'Issues Encountered' heading are the only
comments you MUST not remove
This section is filled out post work as you fill out the checklists.
You SHOULD document any issues encountered and resolved during the sprint.
You MUST document any failed implementations, blockers or missing tests.
-->

- Seeded the event-loop and souvlaki setup directly from Sol's attached
  `souvlaki 0.8.3` / `winit 0.30.13` probe. The released souvlaki path
  attached successfully from an `LSUIElement` bundle with no window.
- The exact macOS escape hatch is two `objc2::msg_send!` sequences:
  `MPRemoteCommandCenter.sharedCommandCenter` →
  `changePlaybackPositionCommand` → `setEnabled:NO`, with `isEnabled`
  read back false; and `MPNowPlayingInfoCenter.defaultCenter` →
  `setNowPlayingInfo:nil`, with `nowPlayingInfo` read back nil.
- Clean shutdown must advance souvlaki's `GLOBAL_METADATA_COUNTER` before
  assigning nil, or its asynchronous artwork callback can republish after
  clear. The spike does this with an empty `set_metadata`, then detaches
  handlers and assigns nil. A real `launchctl bootout` delivered SIGTERM,
  logged the shutdown, read back nil, stopped the process, and left the job
  unloaded.
- The first launch logged every line twice because launchd redirected stderr
  to the same path used by the explicit callback logger. The spike now writes
  once directly to `/tmp/nowplayd-spike.log` and uses stderr only if opening
  that file fails.
- The first human command pass delivered pause/toggle/previous/next but could
  not produce play: the logging-only handler left the published state Playing,
  so macOS correctly kept offering Pause. Callbacks now cross an
  `EventLoopProxy` back to winit's main thread and republish Playing/Paused
  after toggle/play/pause. A pure transition test covers all five transport
  events; the owner rerun remains the live proof of the Play callback.
- With Firefox and a standalone audio player active, macOS displayed the spike
  as a third concurrent Now Playing card with correct metadata, artwork, and
  visible controls. This is useful arbitration evidence: the observed system
  surface is a multi-session picker, not a single exclusive owner. Hardware-key
  routing still needs an uncontested check, but the large ownership matrix in
  the original hypothesis should be reconsidered in the §8 fold.
- The amended owner rerun delivered all five distinct command callbacks,
  including repeated Pause → Play transitions, while competing sessions
  remained present. Its first uninstall exposed a short launchd race: the
  process and artifacts were gone, but an immediate `launchctl print` still
  found the job; a subsequent read found it absent. Install/uninstall now wait
  up to five seconds for confirmed job removal before proceeding or failing.
- The owner has passed presentation, artwork, no-scrubber, and all five command
  checks on the concurrent multi-session surface. Visual clearing after the
  final teardown and the Review Lead's equivalence/baseline ruling remain; no
  claim is made that those governance steps are already complete.
