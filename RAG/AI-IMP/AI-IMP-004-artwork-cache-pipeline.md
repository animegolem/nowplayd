---
node_id: AI-IMP-004
tags:
  - IMP-LIST
  - Implementation
  - artwork
kanban_status: planned
depends_on: [AI-IMP-002, AI-IMP-003]
parent_epic: [[AI-EPIC-001-mpd-macos-now-playing-bridge]]
confidence_score: 0.8
date_created: 2026-08-16
date_completed:
---

# AI-IMP-004-artwork-cache-pipeline

## Summary of Issue #1

Implements SPEC.org §11 FR-4 via the §5.3 atomic cache pipeline. MPD
serves artwork as bytes; souvlaki takes a file URL; the gap needs an
explicit cache lifecycle with stale-art suppression, and souvlaki
0.8.3's artwork path panics on bad input (issue #77) so only
verified-written files may ever reach it. Done-state: artwork for the
§7 fixture appears within 3 s of song change; songs without art show
none (not the previous song's); unit tests cover the cache lifecycle.

### Out of Scope

- Embedded-art fallbacks beyond `albumart` then `readpicture` (no
  folder-image scanning, no network sources).
- Image transcoding/resizing — bytes are cached as-is.
- Cache retention policy beyond current + previous file (no LRU).

### Design/Approach

`src/artwork.rs`: on song-identity change (from the AI-IMP-002 change
summary), fetch `albumart`, fall back to `readpicture`, else None.
Write to cache dir (`~/Library/Caches/nowplayd/`) as temp file +
atomic rename keyed by song identity hash; publish the file URL via
the platform adapter only after rename returns. On None, publish
no-art explicitly (suppression — the adapter republishes metadata
without artwork rather than leaving prior art). Skip refetch when
song identity is unchanged (elapsed/state changes never touch art).

Safety boundary per §5.3 rev 0.4: souvlaki pinned to git rev
`436a5aed` (nil-image guard) — a durable write is NOT a decode
guarantee, so invalid-image fixture coverage is required, and cached
reuse revalidates existence + decodability.

Command-connection fairness per §5.1 rev 0.4: the fetch never holds
an exclusive command-connection borrow across the whole read — the
owner yields between artwork chunks and services pending remote
commands/state refreshes before the next chunk. A slow fetch must not
delay metadata publishing (two-phase publish) NOR remote commands.

### Files to Touch

- `src/artwork.rs`: new.
- `src/mpd/command.rs`: `albumart`/`readpicture` chunked reads over
  the binary primitive.
- `src/platform/mod.rs` + macOS impl: artwork slot in publish path,
  explicit no-art republish.
- `tests/artwork.rs`: cache lifecycle unit tests (temp dir).

### Implementation Checklist

<CRITICAL_RULE>
Before marking an item complete on the checklist MUST **stop** and
**think**. Have you validated all aspects are **implemented** and
**tested**?
</CRITICAL_RULE>

- [ ] Chunked `albumart` + `readpicture` reads with size cap and
      typed errors; fixture tests incl. ACK ("no art") case.
- [ ] Cache write is temp + rename in the same directory; test proves
      no partially-written file is ever at the published path.
- [ ] Song-identity keying: unchanged song → zero fetches (test
      asserts fetch-count).
- [ ] No-art songs: explicit art-less republish; test asserts the
      adapter received a clear-art publish, not silence.
- [ ] Metadata publish is never blocked behind art fetch (two-phase
      publish; test with delayed fake fetch).
- [ ] Invalid-image fixture: corrupt/unsupported bytes are detected
      at our boundary and never reach souvlaki as a published URL
      (test asserts no-art publish instead).
- [ ] Fairness: delayed multi-chunk fake test asserts a remote
      command issued mid-fetch is serviced before the next art chunk.
- [ ] Cache-reuse revalidation: missing or undecodable cached file →
      refetch, not a stale publish (test).
- [ ] Artwork failures logged (FR-8) and never fatal.
- [ ] Live check appended to `RAG/HUMAN-TESTING.md`: fixture track
      art within 3 s; art-less track shows no stale art.
- [ ] Gates green; counts reported.

### Acceptance Criteria

**Scenario:** Fixture artwork lifecycle.
**GIVEN** a queue of [fixture-art track, art-less track].
**WHEN** playback moves to the fixture track.
**THEN** Control Center shows its art within 3 s (§7, human-verified).
**WHEN** playback moves to the art-less track.
**THEN** metadata updates and NO artwork is shown.
**GIVEN** the same song continues playing across state changes.
**THEN** no additional art fetches occur (unit-asserted).

### Issues Encountered

<!--
The comments under the 'Issues Encountered' heading are the only
comments you MUST not remove
This section is filled out post work as you fill out the checklists.
You SHOULD document any issues encountered and resolved during the sprint.
You MUST document any failed implementations, blockers or missing tests.
-->
