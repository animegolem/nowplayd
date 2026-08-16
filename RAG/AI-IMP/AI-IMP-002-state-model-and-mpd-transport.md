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
dependency (rev 0.4): a passing IMP-001 baseline ruling folded into
SPEC.org §8 authorizes this ticket to start (§6).

### Out of Scope

- Any souvlaki/MediaPlayer/macOS code (AI-IMP-003).
- Artwork reads (AI-IMP-004) — the transport exposes the binary-read
  primitive but nothing consumes it here.
- Reconnect/backoff policy (AI-IMP-005) — connection loss surfaces as
  a typed error only.
- Config file parsing (AI-IMP-006) — address comes from a plain
  struct; env/TOML wiring later.

### Design/Approach

`src/state.rs`: `PlayerState` (song identity+tags, play state,
elapsed, duration) as pure data with a `diff`-style change summary
(what changed: metadata / playback / song identity) so the platform
layer can act minimally. `src/mpd/`: wire framing via `mpd_protocol`
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
responses through the mapping layer; the real-MPD smoke test issues
`next` through its OWN second test client (no `mpc` dependency — not
installed on the target machine) and, when the environment supports
it, is RUN and its result reported in the submission — an
ignored-and-uncounted test does not satisfy the done-state.

### Files to Touch

- `Cargo.toml`: tokio, `mpd_protocol = "1.0.3"`, optionally
  `mpd_client = "1.4.1"`.
- `src/lib.rs`: new — exports project modules so `tests/` can import
  them.
- `src/main.rs`: minimal wiring (connect, log state changes).
- `src/state.rs`: new.
- `src/mpd/mod.rs`, `src/mpd/idle.rs`, `src/mpd/command.rs`: new
  (mapping layer only; no `proto.rs` — framing is the dependency's).
- `tests/fixtures/*.txt`, `tests/proto.rs`: new.

### Implementation Checklist

<CRITICAL_RULE>
Before marking an item complete on the checklist MUST **stop** and
**think**. Have you validated all aspects are **implemented** and
**tested**?
</CRITICAL_RULE>

- [ ] `PlayerState` + change-summary type with unit tests for
      metadata-only, playback-only, and song-identity transitions.
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
- [ ] Real-MPD smoke test drives `next` via its own second test
      client (no `mpc`); RUN against local mpd and result reported in
      the submission when the environment supports it — otherwise the
      gap is declared, never counted as passing.
- [ ] Gates green: `cargo build`, `cargo test`,
      `cargo clippy -- -D warnings`; counts reported in submission.

### Acceptance Criteria

**Scenario:** Song change propagates through the transport.
**GIVEN** the daemon is connected to a local mpd with two songs
queued.
**WHEN** the smoke test's own second client issues `next`.
**THEN** the idle connection yields a player event, the command
connection refresh produces a `PlayerState` whose change summary says
song identity changed, and the new title is logged.
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
