---
node_id: AI-IMP-008
tags:
  - IMP-LIST
  - Implementation
  - macos
  - P1
kanban_status: planned
depends_on:
parent_epic: [[AI-EPIC-001-mpd-macos-now-playing-bridge]]
confidence_score: 0.85
date_created: 2026-09-18
date_completed:
---

# AI-IMP-008-artwork-cache-ownership-fence

## P1: retention deletes foreign images in a configured cache directory

Finding NP-1 of Astra's estate review (grisaille
.inbox/tofable/mpd-estate-review-r1.md + nowplayd appendix; evidence
retained under mpd-estate-review-r1-evidence/), source-verified by the
Review Lead: `cleanup_unretained` (src/artwork.rs:289-300, activated
via :269-286) treats EVERY direct-child .jpg/.png in the configured
cache root as owned and deletes all but the active/previous artwork
paths. `src/config.rs:173-186` accepts any non-empty absolute
directory, so pointing `cache_dir` at a shared images folder deletes
unrelated user files during ordinary first artwork publication.
Reproduced by Astra with a real ArtworkCoordinator and a genuine
foreign JPEG (deleted).

### Out of Scope

Retention policy itself (count/age of kept covers). NP-2 (separate
ticket). Any change to artwork lookup or publication.

### Design/Approach

Establish ownership before deletion. Options for the round-1 report
(Code Lead proposes, RL rules): (a) dedicated subdirectory — cache
operates in `<cache_dir>/nowplayd/` it creates and owns; (b) ownership
marker — only delete files matching the daemon's content-addressed
basename scheme; (c) both. Foreign files are NEVER deleted regardless
of extension.

### Files to Touch

src/artwork.rs (cleanup ownership fence); src/config.rs if the
subdirectory shape is ruled; tests: foreign-image preservation across
first and later activations, owned-file cleanup still functioning.

### Implementation Checklist

<CRITICAL_RULE>
Before marking an item complete on the checklist MUST **stop** and **think**. Have you validated all aspects are **implemented** and **tested**?
</CRITICAL_RULE>

- [ ] Ownership boundary implemented per ruled shape.
- [ ] Regression: foreign jpg/png preserved on first and subsequent activations.
- [ ] Regression: owned stale artwork still cleaned.
- [ ] Full gates (fmt, clippy -D warnings, test) reproduced with counts.
- [ ] Atomic submission via channel for review.

### Acceptance Criteria

GIVEN a cache directory containing unrelated images, WHEN the daemon
publishes artwork through repeated activations, THEN no foreign file
is ever deleted AND owned stale artwork continues to be retired.

### Issues Encountered

Not started.
