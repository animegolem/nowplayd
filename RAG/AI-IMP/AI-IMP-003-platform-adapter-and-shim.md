---
node_id: AI-IMP-003
tags:
  - IMP-LIST
  - Implementation
  - macos
  - shim
kanban_status: planned
depends_on: [AI-IMP-001, AI-IMP-002]
parent_epic: [[AI-EPIC-001-mpd-macos-now-playing-bridge]]
confidence_score: 0.7
date_created: 2026-08-16
date_completed:
---

# AI-IMP-003-platform-adapter-and-shim

## Summary of Issue #1

Implements SPEC.org §11 FR-2/FR-3 and the §4 shim: the platform
adapter that publishes `PlayerState` to Now Playing via souvlaki, plus
the target-gated macOS shim providing what souvlaki cannot — true
clearing (nil `nowPlayingInfo`) and disabling
`changePlaybackPositionCommand`. Remote commands flow back to the MPD
command connection. Consumes AI-IMP-001's verified objc2 surface.
Done-state: daemon binary shows live MPD state in Control Center,
media keys drive MPD, no scrubber advertised; live checks queued to
HUMAN-TESTING.

### Out of Scope

- Artwork (AI-IMP-004): adapter publishes metadata with no art.
- Reconnect/clear-on-disconnect policy (AI-IMP-005) — this ticket
  provides the `clear()` primitive; lifecycle decides when.
- Bundle/launchd (AI-IMP-006); dev runs may use a bare binary with
  documented Control Center presentation caveats.

### Design/Approach

`src/platform/mod.rs`: trait-shaped adapter (`publish(state_change)`,
`clear()`, command event stream) so the core stays platform-free;
macOS impl `src/platform/macos/` wraps souvlaki (winit windowless
loop per spike findings) and `src/platform/macos/shim.rs` holds the
ONLY direct MediaPlayer calls, each cited to §4. Threading per spike:
winit loop owns the main thread (macOS requirement); tokio runtime on
a worker; command callbacks cross via channel. Linux leg: souvlaki
MPRIS pass-through with no shim, compile-gated, CI-level only (§4).

### Files to Touch

- `Cargo.toml`: souvlaki 0.8.3 (pinned), winit, objc2 deps
  (target-gated).
- `src/platform/mod.rs`, `src/platform/macos/{mod.rs,shim.rs}`,
  `src/platform/linux.rs`: new.
- `src/main.rs`: main-thread event loop restructure; wire adapter to
  state changes and command events to `CommandConnection`.
- `RAG/HUMAN-TESTING.md`: live checks.

### Implementation Checklist

<CRITICAL_RULE>
Before marking an item complete on the checklist MUST **stop** and
**think**. Have you validated all aspects are **implemented** and
**tested**?
</CRITICAL_RULE>

- [ ] Platform trait + macOS adapter publishing metadata, play state,
      and elapsed from `PlayerState` change summaries (minimal
      updates: metadata-only change does not republish position).
- [ ] `shim.rs`: nil `nowPlayingInfo` clear, exactly the spike's
      verified surface; unit-testable behind a trait where possible.
- [ ] `shim.rs`: `changePlaybackPositionCommand` disabled at attach;
      remains disabled after souvlaki re-attach if any.
- [ ] Command events (toggle/play/pause/next/previous) forwarded to
      MPD; each receipt+result logged (FR-8).
- [ ] Main-thread ownership restructure: winit loop on main, tokio on
      worker, clean channel bridge; no busy-wait (§4 idle-CPU
      invariant).
- [ ] Linux pass-through compiles (`cargo check
      --target x86_64-unknown-linux-gnu` or CI equivalent recorded).
- [ ] Live checks appended to `RAG/HUMAN-TESTING.md`: Control Center
      correctness, each key, no scrubber, competing-player
      preconditions.
- [ ] Gates green; counts reported.

### Acceptance Criteria

**Scenario:** Live MPD state in Control Center (human-verified).
**GIVEN** the daemon runs with mpd playing.
**WHEN** the owner opens Control Center.
**THEN** current title/artist/album and play state are correct within
1 s of a song change (§7) and NO position scrubber is offered.
**WHEN** each media key is pressed.
**THEN** MPD acts accordingly and the command receipt+result is
logged.
**WHEN** `clear()` is invoked (test hook).
**THEN** the Now Playing entry is fully removed, not blanked.

### Issues Encountered

<!--
The comments under the 'Issues Encountered' heading are the only
comments you MUST not remove
This section is filled out post work as you fill out the checklists.
You SHOULD document any issues encountered and resolved during the sprint.
You MUST document any failed implementations, blockers or missing tests.
-->
