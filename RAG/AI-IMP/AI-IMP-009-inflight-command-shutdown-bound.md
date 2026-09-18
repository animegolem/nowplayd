---
node_id: AI-IMP-009
tags:
  - IMP-LIST
  - Implementation
  - macos
kanban_status: planned
depends_on:
parent_epic: [[AI-EPIC-001-mpd-macos-now-playing-bridge]]
confidence_score: 0.78
date_created: 2026-09-18
date_completed:
---

# AI-IMP-009-inflight-command-shutdown-bound

## P2: a stalled command blocks disconnect clearing and clean shutdown

Finding NP-2 of Astra's estate review (grisaille
.inbox/tofable/mpd-estate-review-r1.md, appendix NP-2): after a remote
command is chosen, the worker awaits the complete command/refresh
(src/main.rs:279-280; similar at :308, :352) without servicing the
shutdown watch or the idle-failure channel; command responses
(src/mpd/command.rs:143-145) carry no deadline. A black-holed ACK
retains stale Now Playing state after idle EOF and blocks
signal-driven shutdown. Reproduced with a scripted localhost server
withholding the Next ACK: after shutdown=true the worker stayed
pending; Publish emitted, no Clear.

### Out of Scope

Idle-timeout/reconnect and incoherent-snapshot behavior (previously
fixed; distinct). Command replay (explicitly ruled OUT: an interrupted
mutation is uncertain — never replay it or reuse a potentially
desynchronized socket).

### Design/Approach

Shutdown and fatal idle failure invalidate the in-flight connection
pair; non-idle waits get a bounded deadline. On interruption: drop the
socket pair, clear published state, reconnect fresh (no replay).

### Files to Touch

src/main.rs (select on shutdown/idle-failure across command awaits);
src/mpd/command.rs (deadline); tests with a scripted server: withheld
ACK + shutdown → prompt exit with Clear; withheld ACK + idle EOF →
pair invalidated, no reuse.

### Implementation Checklist

<CRITICAL_RULE>
Before marking an item complete on the checklist MUST **stop** and **think**. Have you validated all aspects are **implemented** and **tested**?
</CRITICAL_RULE>

- [ ] Command awaits interruptible by shutdown and fatal idle failure.
- [ ] Bounded deadline on non-idle response waits.
- [ ] No replay/reuse of an interrupted socket pair (test asserts).
- [ ] Regression: withheld-ACK scenarios emit Clear and exit cleanly.
- [ ] Full gates reproduced with counts; atomic channel submission.

### Acceptance Criteria

GIVEN a server that accepts a command but never ACKs, WHEN shutdown is
signaled or the idle connection fails fatally, THEN the daemon clears
published state and exits (or reconnects) within the bounded deadline,
never replaying the interrupted command.

### Issues Encountered

Not started.
