---
node_id: AI-IMP-002
tags:
  - IMP-LIST
  - Implementation
  - mpd
kanban_status: planned
depends_on: [AI-IMP-001]
parent_epic: [[AI-EPIC-001-mpd-macos-now-playing-bridge]]
confidence_score: 0.85
date_created: 2026-08-16
date_completed:
---

# AI-IMP-002-state-model-and-mpd-transport

## Summary of Issue #1

Implements SPEC.org §11 FR-1 and the data half of FR-2: a pure,
platform-free player state model and the two-connection MPD transport
ruled at §5.1. No such code exists. Done-state: `cargo test` passes a
protocol-fixture suite proving idle events produce correct
`PlayerState` transitions over both connection roles, against a real
local mpd in a smoke test and recorded-response fixtures in unit
tests. `depends_on: AI-IMP-001` is a DESIGN gate, not a code
dependency: SATISFIED at rev 0.5 (spike PASSED via the
concurrent-session equivalence ruling, SPEC.org §8). This sitting
opens on explicit owner authorization per the §6 rev 0.6 per-ticket
model.

### Out of Scope

- Any souvlaki/MediaPlayer/macOS code (AI-IMP-003).
- Artwork reads (AI-IMP-004) — the transport exposes the binary-read
  primitive but nothing consumes it here.
- Reconnect/backoff policy (AI-IMP-005) — connection loss surfaces as
  a typed error only.
- Config file parsing (AI-IMP-006) — address comes from a plain
  struct; env/TOML wiring later.

### Design/Approach

`src/state.rs`: `PlayerState` as pure data carrying the TWO
identities ruled at SPEC.org §5.1.1 — OCCURRENCE identity (queue
`songid`, `Option`, `None` = no current song; not durable across MPD
restarts) and MEDIA key (song URI; durable, the artwork-cache and
reconnect key) — plus tags, play state, elapsed, duration, and a
`diff`-style change summary (metadata / playback / occurrence /
media-key change) so the platform layer can act minimally.

Refresh coherence per §5.1 rev 0.6: `status` + `currentsong` issued
as one grouped command list (`command_list_ok_begin`, separated
responses), then `status.songid` validated against `currentsong.Id`;
mismatched snapshots are discarded and retried — never emitted. `src/mpd/`: wire framing via `mpd_protocol`
**1.0.3** `AsyncConnection` (greeting, escaping, OK/ACK, partial
reads, binary frames — hand-rolled framing declined per §5.1 rev
0.4), optionally `mpd_client` **1.4.1** typed `Command` definitions
on top — but NOT its full `Client` loop (automatic idle/noidle
conflicts with §5.1). Two independent connections per §5.1:
`IdleConnection` sends filtered `idle player mixer options` only;
`CommandConnection` (`status`, `currentsong`, commands, binary reads)
never idles. Local code is limited to mapping framed/typed responses
into `PlayerState` plus project error types. TCP and unix socket,
optional password. Async via tokio. Unit tests feed captured MPD
responses through the mapping layer.

Real-MPD gate (amended rev 0.6 — the "declare the gap" wording is
REMOVED; this gate is mandatory and reproducible): an ISOLATED smoke
harness that never touches the owner's MPD instance, library, queue,
or config. The harness (a) starts its own `mpd` (0.24.14 present at
`/opt/homebrew/bin/mpd`) with a temporary config/database/state
directory, a reserved localhost port or unix socket, and the `null`
audio output; (b) generates two small silent WAV fixtures at runtime
(no committed audio binaries) and loads them via a test client;
(c) connects both nowplayd roles, then issues `next` from a separate
test client (no `mpc` dependency — even though `mpc` exists locally,
the gate must not depend on user tooling); (d) asserts one player
event, a coherent new-song snapshot, and the expected change
summary/title; (e) tears the daemon down and removes temp state even
on failure.

### Files to Touch

- `Cargo.toml` and the tracked root `Cargo.lock` (regenerates with
  the authorized dependency additions — in scope): tokio,
  `mpd_protocol = { version = "=1.0.3", features = ["async"] }`
  (crate has ZERO default features; `AsyncConnection` is gated
  behind `async` — exact pin retained); optionally
  `mpd_client = "=1.4.1"` (typed command/response layer ONLY; full
  `Client` loop prohibited per §5.1).
- `src/lib.rs`: new — exports project modules so `tests/` can import
  them.
- `src/main.rs`: minimal wiring (connect, log state changes).
- `src/state.rs`: new.
- `src/mpd/mod.rs`, `src/mpd/idle.rs`, `src/mpd/command.rs`: new
  (mapping layer only; no `proto.rs` — framing is the dependency's).
- `tests/fixtures/*.txt`, `tests/proto.rs`: new.
- `tests/smoke_mpd.rs`, `tests/support/harness.rs`: new — isolated
  MPD smoke harness (temp state, null output, runtime-generated
  silent WAV fixtures, teardown-on-failure).

### Implementation Checklist

<CRITICAL_RULE>
Before marking an item complete on the checklist MUST **stop** and
**think**. Have you validated all aspects are **implemented** and
**tested**?
</CRITICAL_RULE>

- [ ] `PlayerState` with BOTH §5.1.1 identities + change-summary
      type; unit tests for metadata-only, playback-only, occurrence,
      and media-key transitions.
- [ ] Identity fixtures per §5.1.1: duplicate queue entries sharing
      one URI (distinct occurrence, same media key); queue-id change
      across restart/reload (same media key, new occurrence).
- [ ] Coherence contract: grouped `command_list_ok_begin` refresh
      with `status.songid` == `currentsong.Id` validation;
      fake/fixture test injects a transition between the reads and
      proves no mixed snapshot is ever emitted (discard+retry
      observed).
- [ ] `mpd_protocol` 1.0.3 wired; response-to-`PlayerState` mapping
      layer with fixture tests incl. an ACK error case (framing
      itself is the dependency's, untested here).
- [ ] `src/lib.rs` exports modules; `tests/` imports compile.
- [ ] `CommandConnection`: connect (tcp + unix), optional password,
      `status`, `currentsong`, playback commands
      (toggle/play/pause/next/previous), binary read primitive; never
      sends `idle`.
- [ ] `IdleConnection`: filtered `idle player mixer options` loop
      yielding typed subsystem events; connection drop yields a typed
      error, no retry logic.
- [ ] Wiring in `main.rs`: on idle event, refresh via command
      connection, log the change summary (FR-8 seed).
- [ ] Isolated MPD smoke harness per Design: own mpd + temp state +
      null output + reserved port/socket; runtime-generated silent
      WAV fixtures; `next` issued by its own second test client;
      asserts player event + coherent snapshot + change summary;
      cleans up even on failure. MANDATORY — run and counts reported
      in the submission.
- [ ] Gates green: `cargo build`, `cargo test`,
      `cargo clippy -- -D warnings`; counts reported in submission.

### Acceptance Criteria

**Scenario:** Song change propagates through the transport
(isolated harness — owner's MPD untouched).
**GIVEN** the harness's own mpd (temp state, null output) with the
two generated fixtures queued and nowplayd's both roles connected.
**WHEN** the harness's second client issues `next`.
**THEN** the idle connection yields a player event, the refresh
produces a coherent `PlayerState` (songid-validated) whose change
summary reports an occurrence + media-key change, and the new title
is asserted.
**AND** the test mpd and all temporary state are removed even if the
assertions fail.
**GIVEN** fixture-recorded responses.
**WHEN** the parser consumes them.
**THEN** all fixture tests pass with exact field assertions.

### Issues Encountered

<!--
The comments under the 'Issues Encountered' heading are the only
comments you MUST not remove
This section is filled out post work as you fill out the checklists.
You SHOULD document any issues encountered and resolved during the sprint.
You MUST document any failed implementations, blockers or missing tests.
-->
