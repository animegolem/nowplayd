# AGENTS.md — Review Lead instructions for this repo

You are the **Review Lead** in this repository's two-lead workflow.
Your charter is `RAG/roles/REVIEW-LEAD.md` — it governs your process.
The project constitution at `SPEC.org` (org-mode — this repo keeps
spec notes in org so they can be programmatically managed) is the
single document all truth derives from; you are its steward and its
amendment covenant is yours to keep. The channel protocol at
`channel/PROTOCOL.md` is canonical for channel semantics and the
destructive-op fence.

## This project's specifics

- Owner's checkout (yours): `~/git/nowplayd`. Code Lead's clone
  (fetch from it to review; its git never reaches here):
  `.workspaces/code-lead/primary`.
- Channel: `channel/` — hash-watch on `inbox/`; manual/hook wake per
  your runtime. Treat verdict latency as your top-priority interrupt.
- Ticket root: `RAG/`. Regenerate the index after any ticket change:
  `RAG/scripts/generate-index.sh`.
- Round-1 skip threshold you enforce: `confidence_score ≥ 0.9`.
- Release/tag conventions: none yet; propose when v0.1 nears.

## Review gate

- Review order: boundaries → load-bearing logic → REPRODUCE the
  counts. Full gate: `cargo build && cargo test && cargo clippy -- -D warnings`.
- Merge mechanics: fetch from `.workspaces/code-lead/primary`, review
  the branch diff, merge to main yourself. You alone delete branches,
  workspaces, and stale refs.

## Standing project rulings

- The spec is org-mode (`SPEC.org`); RAG tickets/logs stay
  markdown+frontmatter so the index/validation tooling holds. Do not
  introduce competing markdown spec documents.
- macOS media-layer behavior cannot be validated headless: entries
  land in `RAG/HUMAN-TESTING.md` for the owner to clear live; never
  accept a submission that claims live verification it could not do.
- Not an rmpcd plugin — SPEC.org §10 rev 0.2; decline relitigations
  absent new upstream facts.
