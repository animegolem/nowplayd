---
node_id: AI-IMP-005
tags:
  - IMP-LIST
  - Implementation
  - lifecycle
kanban_status: completed
depends_on: [AI-IMP-002, AI-IMP-003, AI-IMP-004]
parent_epic: [[AI-EPIC-001-mpd-macos-now-playing-bridge]]
confidence_score: 0.8
date_created: 2026-08-16
date_completed: 2026-08-19
---

# AI-IMP-005-reconnect-and-shutdown-lifecycle

## Summary of Issue #1

Implements SPEC.org §11 FR-5: connection-loss handling with backoff
and TRUE clearing on every exit from the "connected with a current
song" state — MPD disconnect, empty queue/no current song, and
graceful daemon shutdown. AI-IMP-002 surfaces drops as typed errors
and AI-IMP-003 provides `clear()`; this ticket supplies the policy
machine. Done-state: kill/restart mpd and the daemon recovers
unaided; Now Playing is verifiably absent while disconnected and
after daemon exit.

### Out of Scope

- launchd KeepAlive semantics (AI-IMP-006) — this ticket makes the
  process itself well-behaved; supervision config is packaging.
- MPD password/auth changes mid-run; address changes require restart.
- Crash-path clearing (SIGKILL): unattainable in-process; note the
  residual behavior in HUMAN-TESTING for the record.

### Design/Approach

`src/lifecycle.rs`: a small connection supervisor implementing the
§5.5 failure matrix (rev 0.4). Both connections are established and
authenticated as ONE transaction; Connected is entered only after
both plus the initial full refresh succeed. Error classification:
transport-level failure (EOF/I-O/malformed protocol) on either role
tears down both, clears once, and begins backoff; MPD command-level
ACK errors log the failure but never tear down a healthy pair or
clear. Backoff: 1 s doubling to 30 s cap, jittered, infinite —
resetting only after a defined healthy interval (ruled: 30 s
connected with a successful refresh), NOT on bare socket handshake,
so a flapping MPD cannot cycle at the floor. On disconnect:
`clear()` before the first backoff sleep. On reconnect: full state
refresh + republish; cached-art reuse revalidates per §5.3 (exists +
decodes), unchanged MEDIA key (§5.1.1) alone insufficient — and queue songid must never be the reconnect comparison, it is not restart-durable. No-song → `clear()`
while staying connected. Shutdown: SIGTERM/SIGINT (and winit exit
path) runs `clear()` before exit — coordinated with the main-thread
event loop from AI-IMP-003.

### Files to Touch

- `src/lifecycle.rs`: new.
- `src/mpd/mod.rs`: typed error classification (malformed-mapping
  variants get a class; §5.5 rev 0.13); `src/platform/mod.rs`:
  typed outcomes + acknowledged-clear event with one-shot ack;
  `src/artwork.rs`: `invalidate_epoch()`; `src/lib.rs`; Cargo files
  if signal/jitter deps are added.
- `src/main.rs`: supervisor owns the run loop; signal wiring.
- `src/mpd/idle.rs` / `command.rs`: expose clean teardown if gaps
  found.
- `tests/lifecycle.rs`: state-machine unit tests with fake
  connections.
- `RAG/HUMAN-TESTING.md`: live checks.

### Implementation Checklist

<CRITICAL_RULE>
Before marking an item complete on the checklist MUST **stop** and
**think**. Have you validated all aspects are **implemented** and
**tested**?
</CRITICAL_RULE>

- [x] Supervisor state machine (connected / reconnecting / shutting
      down): paired transactional establish (no Connected until both
      + initial refresh); unit tests with fake connections for each
      transition.
- [x] Error classification tests: transport failure on either role →
      teardown+clear+backoff; command ACK error → logged, pair stays
      up, no clear.
- [x] Backoff 1 s → 30 s cap with jitter; test asserts the schedule
      shape AND that reset requires the 30 s healthy interval (a
      connect-then-immediate-drop keeps escalating).
- [x] Reconnect cache reuse revalidates cached art (missing /
      undecodable → refetch path from AI-IMP-004); fake test.
- [x] `clear()` on disconnect, before the first backoff sleep (test
      asserts ordering).
- [x] `clear()` on no-current-song while connected; republish on next
      song (fake-connection test).
- [x] Reconnect performs full refresh + republish incl. artwork
      re-publish from cache (media key unchanged AND revalidated per
      §5.3; otherwise refetch).
- [x] SIGTERM/SIGINT → `clear()` → clean exit code 0; works under the
      winit main-thread structure.
- [x] Typed classification preserved through every seam per §5.5
      rev 0.13 (no stringification before the supervisor; ACK =
      healthy-pair only; malformed-mapping tears down; mutating
      transport failure emits outcome AND lifecycle fault).
- [x] Refresh incoherence bounded per §5.1 rev 0.13: exactly three
      attempts → `SnapshotCoherenceExhausted` → teardown/clear/
      backoff; fixture asserts count and routing.
- [x] Acknowledged production clear (one-shot ack) awaited before
      first backoff sleep and before clean exit; SIGTERM/SIGINT
      route through it, exit 0; clear failure terminal and honest.
- [x] `invalidate_epoch()` on disconnect/no-song/shutdown; stale job
      completions discarded; reconnect re-runs cache
      lookup/decode; idle task join handle owned and
      aborted/awaited on every teardown (test: no stale idle role
      after reconnect).
- [x] Deterministic timing per §5.5 rev 0.13: injected
      clock/sleeper/RNG; 1,2,4,8,16,30 cap with jitter
      [nominal/2, nominal]; reset only on coherent refresh ≥30 s
      connected; bounds asserted without wall-clock sleeps.
- [x] Runtime inputs injected (ConnectionConfig, cache root,
      event/log sink) with call-site defaults; interim stderr sink
      retained for 006 to swap.
- [x] All transitions logged with reason (FR-8).
- [x] Live checks PROPOSED IN THE SUBMISSION (HUMAN-TESTING is the
      Review Lead's): kill mpd (one true clear before backoff),
      restart (returns unaided with revalidated art), `mpc clear`,
      SIGTERM/SIGINT acknowledged clear + exit 0.
- [x] Gates green; counts reported.

### Acceptance Criteria

**Scenario:** MPD restart under a live session.
**GIVEN** the daemon is publishing a playing track.
**WHEN** mpd is killed.
**THEN** the Now Playing entry is removed and reconnect attempts back
off toward 30 s.
**WHEN** mpd returns with the queue restored and playing.
**THEN** the entry reappears with correct state and artwork, no
daemon restart.
**WHEN** the daemon receives SIGTERM.
**THEN** the entry is removed and the process exits 0.

### Issues Encountered

<!--
The comments under the 'Issues Encountered' heading are the only
comments you MUST not remove
This section is filled out post work as you fill out the checklists.
You SHOULD document any issues encountered and resolved during the sprint.
You MUST document any failed implementations, blockers or missing tests.
-->

- Implemented the paired supervisor with typed command/idle/artwork
  faults, acknowledged main-thread clear, joined idle teardown,
  deterministic backoff policy, signal shutdown, and conservative
  one-clear latching across repeated failed reconnects.
- Existing cache corruption coverage proves reconnect-style reuse
  re-decodes before publication; epoch invalidation coverage proves
  old blocking completions are deterministically stale.
- Gate: build passed; 62 tests passed; clippy passed with warnings
  denied. No wall-clock sleeps are used in lifecycle tests.
- Live proposal reserved for the wave submission: kill/restart MPD,
  `mpc clear`, SIGTERM and SIGINT acknowledged-clear exit, artwork
  cache revalidation, and the >60-second quiet soak.
- Round-2 review found that macOS consumed `WorkerEvent::Fatal` by exiting the
  event loop, then returned `Ok` from `run()`; with launchd
  `SuccessfulExit=false`, that incorrectly suppressed process-failure restart.
  The focused follow-up preserves the fatal reason through event-loop shutdown
  and returns it as a process error afterward. A seam test distinguishes the
  fatal result from ordinary `ShutdownComplete`/test-hook exits. Round-2 gate:
  72 tests passed; build, clippy with warnings denied, Linux cross-check, and
  changed-ticket validation passed.


## Closure (2026-08-19)

Merged to main (`0253e49` wave tip; installer argv repair `1466f43`).
Owner ruled a PROVISIONAL live pass sufficient to close during
development; the full Wave 4 combined matrix is a mandatory pre-1.0
blocker recorded in RAG/HUMAN-TESTING.md.
