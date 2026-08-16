---
node_id: AI-IMP-005
tags:
  - IMP-LIST
  - Implementation
  - lifecycle
kanban_status: planned
depends_on: [AI-IMP-002, AI-IMP-003]
parent_epic: [[AI-EPIC-001-mpd-macos-now-playing-bridge]]
confidence_score: 0.8
date_created: 2026-08-16
date_completed:
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

`src/lifecycle.rs`: a small connection supervisor owning both MPD
connections as a pair (a drop of either tears down and reconnects
both — split-brain between idle and command connections is worse
than a clean cycle). Backoff: 1 s doubling to 30 s cap, jittered,
infinite; each transition logged (FR-8). On disconnect: `clear()`
immediately. On reconnect: full state refresh + republish. No-song
(`status` with no `currentsong`, e.g. cleared queue) → `clear()`
while staying connected. Shutdown: SIGTERM/SIGINT handler (and winit
exit path) runs `clear()` before process exit — coordinated with the
main-thread event loop from AI-IMP-003.

### Files to Touch

- `src/lifecycle.rs`: new.
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

- [ ] Supervisor state machine (connected / reconnecting / shutting
      down) with paired-connection teardown; unit tests with fake
      connections for each transition.
- [ ] Backoff 1 s → 30 s cap with jitter; test asserts the schedule
      shape and reset-on-success.
- [ ] `clear()` on disconnect, before the first backoff sleep (test
      asserts ordering).
- [ ] `clear()` on no-current-song while connected; republish on next
      song (fake-connection test).
- [ ] Reconnect performs full refresh + republish incl. artwork
      re-publish from cache (no refetch if identity unchanged).
- [ ] SIGTERM/SIGINT → `clear()` → clean exit code 0; works under the
      winit main-thread structure.
- [ ] All transitions logged with reason (FR-8).
- [ ] Live checks appended to `RAG/HUMAN-TESTING.md`: kill mpd (entry
      disappears), restart mpd (entry returns unaided), `mpc clear`
      (entry disappears), SIGTERM daemon (entry disappears).
- [ ] Gates green; counts reported.

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
