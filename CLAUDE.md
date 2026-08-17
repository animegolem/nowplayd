# nowplayd

MPD → macOS Now Playing bridge. A standalone daemon that connects to MPD as a
peer client (`idle` loop) and publishes playback state to
`MPNowPlayingInfoCenter` / handles `MPRemoteCommandCenter` commands, so media
keys, Control Center, and AirPods gestures control MPD regardless of which
frontend is in use. Rust; `souvlaki` is the platform layer.

**The constitution is `SPEC.org`** (org-mode — this repo keeps spec notes in
org so they can be programmatically managed). It is the single document all
truth derives from; do not create competing markdown spec documents.

## Two-lead workflow

This repo runs the review-gated two-lead pattern (see `RAG/roles/`):

- **Review Lead** (Fable/Claude, long-context seat, owner's checkout,
  `CLAUDE.md`): rulings, tickets, briefs, verdicts, merges, ALL destructive git.
- **Code Lead** (Sol/Codex, `AGENTS.md`): round-1 source review from the
  canonical project task, then authorized implementation in an isolated clone
  under `.workspaces/code-lead/<id>` on an `imp/*` branch, one atomic commit
  per ticket, submissions via the channel.
- Channel: `channel/` (local, gitignored) per `channel/PROTOCOL.md` — also the
  canonical home of the destructive-op fence.

If you are Claude Code reading this in the **canonical checkout**: this
checkout belongs to the owner and the Review Lead. Never force-push, reset,
`git clean`, or delete branches/workspaces here; implementation work happens
in the Code Lead clone.

## Work tracking (RAG/)

- Templates in `RAG/templates/` are mandatory. Tickets/logs stay
  markdown+frontmatter (the org move covers the spec only).
- Run `RAG/scripts/generate-index.sh` after ANY ticket change; commit
  `INDEX.md` with it. Never hand-edit `INDEX.md`.
- `RAG/scripts/validate-tickets.sh --changed` is the LOUD gate before any
  submission.
- Check checklist items only after implemented AND validated; fill "Issues
  Encountered" honestly; end sessions with an `RAG/AI-LOG/` entry.
- Design questions for the Review Lead: append to `RAG/DESIGN-QUEUE.md`.
- Anything needing live macOS verification: append to `RAG/HUMAN-TESTING.md`.

## Build / validation

- Gates: `cargo build` / `cargo test` / `cargo clippy -- -D warnings`.
- macOS media-layer behavior (media keys, Control Center) cannot be validated
  headless — queue it for the owner, never claim it verified.

## Related

- `~/git/_notmyrepos_/rmpc` — upstream rmpc reference clone (rmpcd plugin
  question resolved: SPEC.org §10 rev 0.2).
- `~/git/_notmyrepos_/RMPC-Auto-Theme` — the owner's separate theming project;
  out of scope here.
