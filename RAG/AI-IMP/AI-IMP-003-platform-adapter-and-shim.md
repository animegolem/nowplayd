---
node_id: AI-IMP-003
tags:
  - IMP-LIST
  - Implementation
  - macos
  - shim
kanban_status: in-progress
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
- Reconnect/clear-on-disconnect policy, backoff, and paired-teardown
  supervision (AI-IMP-005) — this ticket provides the `clear()`
  primitive; lifecycle decides when. (Amended rev 0.10:) M3 DOES own
  COMMAND-ROLE LIVENESS per §5.1 rev 0.10 — the narrow per-use
  staleness/ping/reconnect contract — because M3's acceptance
  requires surviving an ordinary listening session; the supervisor
  builds on top of it in IMP-005.
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

- `Cargo.toml`: `souvlaki = { git = "https://github.com/animegolem/souvlaki",
  rev = "ba2bf7653ace6ae22880ee7eeed623fee938cacf",
  default-features = false, features = ["use_zbus"] }` (rev 0.9 —
  owned fork: upstream 436a5aed + the 3-line macOS compile fix,
  §5.3) — the pin defaults to `use_dbus`, so
  `default-features = false` is MANDATORY or native libdbus stays
  enabled. `winit` 0.30 and `objc2` deps
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

- [x] Platform trait takes full `PlayerState` snapshots; macOS
      adapter restores playback state + elapsed after every
      `set_metadata`/artwork-phase publish (§5.2); spy test asserts
      the call order.
- [x] `shim.rs`/adapter: `clear()` per §5.2 rev 0.8 — generation-
      advance then native nil, controls STAY ATTACHED (the spike's
      detach step is adapter-lifetime, not clear); typed errors, no
      `assert!`/`expect`; delayed-art fake test asserts a clear can
      never be followed by a stale artwork publish.
- [x] Attach contract: every attach = souvlaki attach → immediately
      disable `changePlaybackPositionCommand`; spy asserts BOTH
      ordered sequences (attach and clear).
- [x] `shim.rs`: `changePlaybackPositionCommand` disabled at attach;
      remains disabled after souvlaki re-attach if any.
- [x] Local command enum of EXACTLY toggle/play/pause/next/previous
      (souvlaki seek/volume/stop events cannot leak); tests prove all
      five mappings.
- [x] Remote-command state loop per §5.2 rev 0.8: successful command
      → immediate coherent `refresh()` → FULL publish (never waiting
      on idle alone); ACK failure logged as failed result, no
      optimistic state; test asserts the success→refresh→publish
      order.
- [x] Receipt+result as TYPED outcomes; `main` emits via the current
      stderr record (FR-8 interim seam — IMP-006 swaps the sink to
      tracing); tests assert semantic outcomes, not stderr text.
- [x] Multi-artist projection per §5.2 rev 0.8: ordered join with
      `"; "`, empty → `None`; mapping test uses the two-artist
      fixture shape.
- [x] Main-thread ownership restructure: winit loop on main, tokio on
      worker, clean channel bridge; no busy-wait (§4 idle-CPU
      invariant).
- [x] Command-role liveness per §5.1 rev 0.10: at-use staleness check
      (50 s threshold) → ping-validate → reconnect+re-auth; transport
      fixture with test clock proves viability across a simulated
      timeout interval; idle future never cancelled.
- [x] Retry discipline: non-mutating ops retry once after reconnect;
      MUTATING commands never blindly retried — double-execution
      guard test proves a failed `next` is logged as a failed result
      and not reissued.
- [x] Linux pass-through (souvlaki `use_zbus`, no default features)
      compiles: run `rustup target add x86_64-unknown-linux-gnu`
      (target not presently installed), then
      `cargo check --target x86_64-unknown-linux-gnu`; exact command
      + result reported in the submission.

- [x] Live-check matrix PROPOSED IN THE SUBMISSION (per the
      CODE-LEAD charter the Code Lead does not touch
      `RAG/HUMAN-TESTING.md`; the Review Lead appends the accepted
      matrix): live MPD metadata/playback after a metadata
      transition, all five keys, no scrubber, true clear via the
      adapter test hook — under NORMAL competing sessions (§7) — AND
      a QUIET SOAK exceeding 60 s of listening before a natural
      track transition (the rev 0.10 failure class).
      Artwork-phase elapsed-persistence check moved to AI-IMP-004.
- [x] Temporary UNCOMMITTED LSUIElement dev bundle (assembled from
      the root release binary) authorized for the live run, with
      explicit teardown; a bare-binary presentation miss is NOT a
      platform failure; production installers stay in IMP-006.
- [x] Gates green; counts reported.

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

- The constitutionally pinned upstream souvlaki revision failed its first real
  macOS build with three unresolved `nil` references. Work stopped before any
  dependency workaround; rev 0.9 replaced it with the owner-authorized
  `animegolem/souvlaki@ba2bf765` fork containing only the three-line raw-pointer
  correction. A clean Cargo fetch then compiled on macOS and Linux.
- The Linux standard-library target was not installed at sitting start, as
  predicted by round 1. `rustup target add x86_64-unknown-linux-gnu` succeeded,
  followed by the exact cross-target check with `use_zbus` only.
- A background launch directly from the shell was reaped with its terminal
  process group. The authorized ad-hoc signed LSUIElement bundle stays alive
  when started through macOS Launch Services; that is the owner live-gate path.
- The first owner live run falsified the submission: MPD's default 60-second
  connection timeout reaped the quiet command role, so the next natural track
  transition exited the worker. Rev 0.10 assigned M3 a per-use 50-second
  staleness check, ping validation, reconnect/re-auth, and asymmetric retry
  discipline. The repair also isolates the idle waiter in its own task so
  command-side work never cancels its in-flight protocol future.
- The first delayed inspection of the clear hook found that native nil had
  succeeded, but a later MPD/remote event performed another full publish and
  recreated the card. The M3-only hook now latches publication off after clear,
  leaving the adapter and remote controls attached while making the primitive's
  persistent absence observable; production reconnect publication remains
  owned by IMP-005 and is unchanged.
- The owner completed the Control Center matrix under normal competing
  sessions: correct title/artist/album/playback, no scrubber, UI and hardware
  play/pause, next, previous, a quiet soak beyond 60 seconds followed by a
  natural transition with the process surviving, and true clear. The latched
  clear remained absent across two later MPD events while the process stayed
  alive. One ambiguous observation had Firefox resume near the clear; M3's log
  contained no play/toggle callback, IINA did not resume, and exact causality
  could not be reconstructed. The owner ruled it non-blocking unless it recurs.
- Teardown completed: the exact clear-test process was terminated, MPD was
  restored to its prior paused state, and the temporary bundle was moved from
  Applications to Trash (recoverable).
- The exact repair gate passed at the rebased branch tip: 29 tests passed
  (14 library, 14 protocol, 1 isolated-MPD smoke), 0 failed, 0 ignored;
  clippy passed with warnings denied; the Linux cross-target check passed; and
  ticket validation reported 8 files checked with 0 errors and 0 warnings.
