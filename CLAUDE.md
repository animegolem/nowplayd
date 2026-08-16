# nowplayd

MPD → macOS Now Playing bridge. A standalone daemon that connects to MPD as a
peer client (`idle` loop) and publishes playback state to
`MPNowPlayingInfoCenter` / handles `MPRemoteCommandCenter` commands, so media
keys, Control Center, and AirPods gestures control MPD regardless of which
frontend is in use. Rust; `souvlaki` is the expected platform layer (which also
gives MPRIS on Linux for free).

## Spec

The normative spec lives in `SPEC.org` — **org-mode, not markdown**. This repo
is part of the move to org for spec notes so they can be programmatically
managed. Do not create competing markdown spec documents; deferred/descoped
features carry self-contained scope in SPEC.org.

## Work tracking (RAG/)

- Templates in `RAG/templates/` are mandatory for epics, IMP tickets, and
  session logs. Tickets/logs stay markdown+frontmatter (the org move covers the
  spec only).
- Run `./RAG/scripts/generate-index.sh` after ANY ticket change and commit
  `INDEX.md` with it. Never hand-edit `INDEX.md`.
- Check checklist items only after the item is implemented AND validated.
- Discuss IMP breakdowns with the owner before writing ticket files.
- Fill "Issues Encountered" honestly — it is the handoff the diff can't give.
- End every implementation session with an `RAG/AI-LOG/` entry from the
  template; its Next Steps must let a fresh context pick up cold.
- One commit per closed ticket; commit messages explain the decision, not just
  the change.

## Build / validation

- `cargo build` / `cargo test` / `cargo clippy -- -D warnings` are the gates.
- macOS integration (Now Playing registration, media keys) cannot be fully
  validated headless — flag anything needing a live manual check in the ticket
  rather than claiming it verified.

## Related

- `~/git/_notmyrepos_/rmpc` — reference clone of upstream rmpc. The user has a
  fork of rmpc (separate concern: rebasing it on upstream and checking whether
  upstream has grown a plugin layer, see rmpcd/). nowplayd is deliberately its
  own project, frontend-agnostic.
