# Human Testing Queue

Behavior that cannot be validated headless (macOS media keys, Control Center,
lock screen, AirPods). Tickets append here; the owner clears entries after
live verification.

## Completed

### AI-IMP-001 — spike architecture gate (PASSED 2026-08-16)

Owner-verified live, ruling folded into SPEC.org §8 (rev 0.5):

- Presentation + routing verified under CONCURRENT sessions (Firefox +
  standalone AAC player active): spike appeared as a third card with
  title/album/artwork, no scrubber, all five commands
  (`toggle`/`play`/`pause`/`next`/`previous`) logged with timestamps,
  including repeated pause→play transitions after the handler amendment.
  Equivalence ruling: this supersedes the quit-everything baseline (strictly
  stronger conditions).
- True clearing on SIGTERM verified by native readback AND visually by the
  owner: the card disappears from the Now Playing surface.
- Teardown left no live agent, bundle, plist, or log (with the ~5 s launchd
  job-removal race accounted for).

## Open (observational — cannot fail any ticket)

### Arbitration characterization (enrichment for AI-IMP-003)

Repeat spike (or later daemon) install/inspect/uninstall while Music.app is
(a) stopped, (b) paused, (c) playing before activation; for each, change
Music to each other state after activation. Record verbatim which source owns
Control Center at each transition and what action lets nowplayd regain it.
Note: §8's multi-session-picker finding predicts the surface shows all
sessions concurrently; these runs characterize which card is DEFAULT/focused.

## Standing queue rules

- Entries must state competing Now Playing session preconditions and
  distinguish mandatory-pass baselines from observational characterization
  (SPEC.org §7).
- Never claim live verification in a submission; queue it here instead.
