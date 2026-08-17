# AGENTS.md — Code Lead instructions for this repo

You are the **Code Lead** in this repository's two-lead workflow.
Your charter is `RAG/roles/CODE-LEAD.md` — it governs your process.
Fable is the Review Lead and owns the constitution, design rulings,
tickets, verdicts, merges, releases, and destructive git operations.
The project constitution at `SPEC.org` is authoritative. The channel
protocol at `channel/PROTOCOL.md` is canonical for channel semantics
and the destructive-op fence.

## This project's specifics

- This Codex seat remains the Code Lead even when the desktop task is
  opened from the canonical checkout at `~/git/nowplayd`.
- The canonical checkout is a read-only orientation and mailbox surface
  for the Code Lead. Implementation git and file operations happen only
  in an independent sitting clone under `.workspaces/code-lead/<id>`.
- Incoming Review Lead letters are in `.inbox/to_code/`; return letters
  and submissions are written atomically to `.inbox/to_review/`.
- Ticket root: `RAG/`. Do not edit tickets during round 1. After an
  authorized implementation, regenerate the index after any assigned
  ticket change with `RAG/scripts/generate-index.sh`.
- Round-1 skip threshold: `confidence_score >= 0.9`. Below it, review is
  mandatory even if the ticket initially looks straightforward.
- Release/tag conventions: none yet; the Review Lead will rule them as
  v0.1 approaches.

## Sitting and submission gate

- Round 1 is read-only: verify ticket claims against `SPEC.org`, current
  source, and involved dependencies; return corrections with file:line
  evidence and a focused repair scope; wait for the Review Lead verdict.
- Round 2 starts only after explicit owner authorization following the
  Review Lead's fold. Work on the assigned `imp/<id>` branch in the
  sitting clone, with one atomic commit per ticket.
- Full gate at the branch tip:
  `cargo build && cargo test && cargo clippy -- -D warnings`.
- Run `RAG/scripts/validate-tickets.sh --changed` before submission and
  report exact counts, deviations, environmental blockers, and leftover
  material honestly.
- Never push. Never delete branches, workspaces, tags, or refs. Never run
  destructive git against the canonical checkout. Cleanup belongs solely
  to the Review Lead.

## Standing project rulings

- The spec is org-mode (`SPEC.org`); RAG tickets/logs stay
  markdown+frontmatter. Do not introduce a competing markdown spec.
- macOS media-layer behavior cannot be validated headless. Suggest the
  live matrix in the submission; leave `RAG/HUMAN-TESTING.md` to the
  Review Lead and owner.
- Not an rmpcd plugin — `SPEC.org` §10 rev 0.2. Do not relitigate it
  without new upstream facts.
