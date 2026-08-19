---
node_id: AI-IMP-007
tags:
  - IMP-LIST
  - Implementation
  - artwork
  - macos
kanban_status: completed
depends_on: [AI-IMP-004]
parent_epic: [[AI-EPIC-001-mpd-macos-now-playing-bridge]]
confidence_score: 0.75
date_created: 2026-08-17
date_completed:
---

# AI-IMP-007-artwork-continuity

## Summary of Issue #1

Owner ruling from the M4 live gate (SPEC.org §5.3 rev 0.12): the
card visibly drops and reloads artwork on every publish. Two
mechanisms: (a) pause/play — every full publish passes through
souvlaki `set_metadata`, which rebuilds the dictionary WITHOUT the
artwork key and re-queues the async image load even for an identical
URL; (b) track changes — the rev 0.11 art-less-first policy clears
art that a cache-hit is about to re-show (same-album skips flicker
on every press). Done-state: pause/play and same-art transitions
show NO visible art drop in the owner live check; different-art
transitions swap on arrival; resolved no-art clears; all existing
guards (stale-discard, true clear) still test green.

### Out of Scope

- Any MPD-side change — fetch/cache/assembly from IMP-004 is
  untouched.
- Lifecycle supervision (AI-IMP-005), config/packaging (AI-IMP-006).
- Cross-fade/animation polish — continuity only.
- Linux MPRIS flicker behavior (observe only; fix if free via the
  shared shape, do not chase).

### Design/Approach

Per §5.3 rev 0.12 AS AMENDED rev 0.13 (round-1 found the held-URL
mechanism non-executable — souvlaki set_metadata assigns art-less
immediately):

0. Fork feature commit in animegolem/souvlaki (§5.3 rev 0.13):
   macOS metadata op preserves current native artwork while a
   non-null URL loads, atomic swap on guarded completion,
   synchronous removal on resolved no-art; generation guard kept.
   Branch pushed under the owner's range authorization; new
   immutable rev pinned here (Cargo.toml + Cargo.lock in scope).
   Fork diff is part of wave review.

1. Publication intent enum selected by the worker (§5.3 rev 0.13):
   FullMetadata vs PlaybackOnly; occurrence-only with no projected
   delta makes no platform call; clear is never a continuity
   publish. Playback-only diffs (same media key, playback/elapsed change):
   macOS impl takes souvlaki's merge path (`set_playback` /
   `set_playback_progress` copy the prior dictionary) instead of
   `set_metadata` — artwork key survives natively. The adapter
   boundary still receives full snapshots (rev 0.8 contract
   refined, not reversed); the macOS impl selects the call path
   from the diff.
2. Media-key changes: publish new metadata WITH the outgoing cover
   URL retained (hold), then swap when the guarded artwork phase
   resolves — same art: no-op; different art: swap; resolved
   no-art: art-less republish then. The rev 0.11 stale-discard
   guard is unchanged: holding is not-yet-removing current art,
   never publishing a stale fetch as new.
3. True `clear()` paths unchanged and re-asserted.

### Files to Touch

- fork: `animegolem/souvlaki` feature branch (preserve/swap).
- `Cargo.toml` + root `Cargo.lock`: new fork pin.
- `src/platform/mod.rs`: publication intent shape, hold/swap policy.
- `src/platform/macos/mod.rs`: merge-path call selection.
- `src/platform/linux.rs`: keep compiling with the shared shape.
- `src/main.rs`: worker publish sites if the event shape changes.
- `tests/artwork.rs` / platform tests: continuity cases.

### Implementation Checklist

<CRITICAL_RULE>
Before marking an item complete on the checklist MUST **stop** and
**think**. Have you validated all aspects are **implemented** and
**tested**?
</CRITICAL_RULE>

- [x] Fork feature commit implemented, reviewed diff, new rev pinned;
      `cargo check` from a clean fetch.
- [x] Publication intent enum with the §5.3 rev 0.13 mapping incl.
      occurrence-only no-call rule.
- [x] Playback-only diff routes to the merge path; spy test asserts
      no `set_metadata` call and artwork key untouched; Stopped and
      absent-progress cases route through the art-preserving full
      path (tests for Playing/Paused with progress, Stopped, absent).
- [x] Media-key change: continuity contract is OBSERVABLE — spy/fork
      tests assert the prior native artwork key remains present
      until replacement ("no intermediate art-less native state");
      different-art swaps on arrival; resolved no-art removes
      synchronously.
- [x] Stale-discard (delayed A→B) and true-clear tests still green,
      re-asserted against the new paths.
- [x] Elapsed/state correctness preserved through merge-path
      publishes (the rev 0.8 dictionary-wholesale hazard cannot
      reappear via a missed field).
- [x] Linux leg compiles; behavior observed and noted, not chased.
- [x] Live matrix proposed in the submission: pause/play with NO
      art drop; same-album skip with NO art drop; different-album
      skip swaps cleanly; art-less transition lingers then clears;
      soak rule applies.
- [x] Gates green; counts reported.

### Acceptance Criteria

**Scenario:** Continuity under the owner's live check.
**GIVEN** the daemon publishing a track with artwork.
**WHEN** the owner pauses and resumes.
**THEN** the artwork never visibly disappears.
**WHEN** the owner skips within the same album.
**THEN** the artwork never visibly disappears.
**WHEN** the owner skips to a different-art track.
**THEN** the old art holds until the new art arrives, then swaps.
**WHEN** the owner plays an art-less track.
**THEN** prior art lingers only until the fetch resolves no-art,
then the card shows no art.
**WHEN** MPD disconnects or the queue empties.
**THEN** true clearing behaves exactly as before.

### Issues Encountered

<!--
The comments under the 'Issues Encountered' heading are the only
comments you MUST not remove
This section is filled out post work as you fill out the checklists.
You SHOULD document any issues encountered and resolved during the sprint.
You MUST document any failed implementations, blockers or missing tests.
-->

- Fork commit `5d237b62980b3f7f246c04bb2349c7b8b8360542`
  was pushed to `animegolem/souvlaki` branch
  `codex/nowplayd-artwork-continuity` under the explicit range
  authorization and pinned immutably here. Clean-fetch `cargo check`
  and `cargo test --lib` passed. The fork's pre-existing doctest opens
  its media event-loop example and does not terminate unattended, so
  the full `cargo test` doctest phase was interrupted after the library
  tests passed; no fork test failure was observed.
- nowplayd gate: build passed; 51 tests passed; clippy passed with
  warnings denied. Linux cross-target `cargo check` passed.
- Live proposal reserved for the wave submission: pause/play and
  same-album continuity; different-album swap; art-less linger then
  clear; true-clear regression and soak.


## Closure (2026-08-19)

Merged to main (`0253e49` wave tip; installer argv repair `1466f43`).
Owner ruled a PROVISIONAL live pass sufficient to close during
development; the full Wave 4 combined matrix is a mandatory pre-1.0
blocker recorded in RAG/HUMAN-TESTING.md.
