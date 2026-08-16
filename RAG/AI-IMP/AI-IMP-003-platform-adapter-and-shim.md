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

`src/platform/mod.rs`: trait-shaped adapter taking a coherent FULL
`PlayerState` snapshot per §5.2 rev 0.4 (souvlaki's `set_metadata`
replaces the whole dictionary — no merging; the core's change summary
may optimize WHEN to publish, never WHAT), plus `clear()` and a
command event stream. macOS impl `src/platform/macos/` wraps souvlaki
(pinned git rev per §5.3) with the winit-main / tokio-worker /
channel-bridge structure AS VERIFIED BY IMP-001 — that shape is the
spike's hypothesis to prove, and this ticket consumes its recorded
findings, adjusting if the spike proved otherwise.
`src/platform/macos/shim.rs` holds the ONLY direct MediaPlayer calls,
each cited to §4. `clear()` follows the §5.2 sequence: advance
souvlaki's metadata generation (`set_metadata(default())`) THEN
native nil — bare nil loses the race with in-flight artwork loads.
Linux leg: souvlaki MPRIS pass-through with no shim, feature
`use_zbus` (avoids the native libdbus build dependency), gate run
locally via `rustup target add x86_64-unknown-linux-gnu` +
`cargo check --target x86_64-unknown-linux-gnu` (no CI workflow in
v1; the check result is reported in the submission).

### Files to Touch

- `Cargo.toml`: `souvlaki = { git = "https://github.com/Sinono3/souvlaki",
  rev = "436a5aedd85a755ba119916ba4504fb866803797",
  default-features = false, features = ["use_zbus"] }` — the pin
  defaults to `use_dbus`, so `default-features = false` is MANDATORY
  or native libdbus stays enabled. `winit` 0.30 and `objc2` deps
  target-gated to macOS (Linux needs neither). Add `sync` to the
  tokio features (the winit-main / tokio-worker channel bridge needs
  it).
- Root `Cargo.lock`: in scope (git pin + zbus graph must lock).
- `src/lib.rs`: export the new platform module.
- `src/platform/mod.rs`, `src/platform/macos/{mod.rs,shim.rs}`,
  `src/platform/linux.rs`: new.
- `src/main.rs`: main-thread event loop restructure; wire adapter to
  state changes and command events to `CommandConnection`.
- (HUMAN-TESTING ownership: Review Lead only — matrix proposed in the submission.)

### Implementation Checklist

<CRITICAL_RULE>
Before marking an item complete on the checklist MUST **stop** and
**think**. Have you validated all aspects are **implemented** and
**tested**?
</CRITICAL_RULE>

- [ ] Platform trait takes full `PlayerState` snapshots; macOS
      adapter restores playback state + elapsed after every
      `set_metadata`/artwork-phase publish (§5.2); spy test asserts
      the call order.
- [ ] `shim.rs`/adapter: `clear()` per §5.2 rev 0.8 — generation-
      advance then native nil, controls STAY ATTACHED (the spike's
      detach step is adapter-lifetime, not clear); typed errors, no
      `assert!`/`expect`; delayed-art fake test asserts a clear can
      never be followed by a stale artwork publish.
- [ ] Attach contract: every attach = souvlaki attach → immediately
      disable `changePlaybackPositionCommand`; spy asserts BOTH
      ordered sequences (attach and clear).
- [ ] `shim.rs`: `changePlaybackPositionCommand` disabled at attach;
      remains disabled after souvlaki re-attach if any.
- [ ] Local command enum of EXACTLY toggle/play/pause/next/previous
      (souvlaki seek/volume/stop events cannot leak); tests prove all
      five mappings.
- [ ] Remote-command state loop per §5.2 rev 0.8: successful command
      → immediate coherent `refresh()` → FULL publish (never waiting
      on idle alone); ACK failure logged as failed result, no
      optimistic state; test asserts the success→refresh→publish
      order.
- [ ] Receipt+result as TYPED outcomes; `main` emits via the current
      stderr record (FR-8 interim seam — IMP-006 swaps the sink to
      tracing); tests assert semantic outcomes, not stderr text.
- [ ] Multi-artist projection per §5.2 rev 0.8: ordered join with
      `"; "`, empty → `None`; mapping test uses the two-artist
      fixture shape.
- [ ] Main-thread ownership restructure: winit loop on main, tokio on
      worker, clean channel bridge; no busy-wait (§4 idle-CPU
      invariant).
- [ ] Linux pass-through (souvlaki `use_zbus`, no default features)
      compiles: run `rustup target add x86_64-unknown-linux-gnu`
      (target not presently installed), then
      `cargo check --target x86_64-unknown-linux-gnu`; exact command
      + result reported in the submission.

- [ ] Live-check matrix PROPOSED IN THE SUBMISSION (per the
      CODE-LEAD charter the Code Lead does not touch
      `RAG/HUMAN-TESTING.md`; the Review Lead appends the accepted
      matrix): live MPD metadata/playback after a metadata
      transition, all five keys, no scrubber, true clear via the
      adapter test hook — under NORMAL competing sessions (§7).
      Artwork-phase elapsed-persistence check moved to AI-IMP-004.
- [ ] Temporary UNCOMMITTED LSUIElement dev bundle (assembled from
      the root release binary) authorized for the live run, with
      explicit teardown; a bare-binary presentation miss is NOT a
      platform failure; production installers stay in IMP-006.
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
