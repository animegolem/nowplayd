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

### AI-IMP-003 — platform adapter live matrix (PASSED round 2, 2026-08-16)

Owner-verified under normal competing sessions (dev bundle, teardown
completed):

- Title / ordered artists / album / play state correct within 1 s of
  transitions; artwork intentionally absent (IMP-004); no scrubber.
- All five commands acted on MPD with paired receipt/result records.
- Rev 0.10 soak: PID survived >60 s quiet + the subsequent natural
  transition (the command-timeout failure class, now fixed by the
  §5.1 liveness contract).
- True clear removed the row with the process alive and controls
  attached; two later MPD player events did NOT recreate it
  (clear-latch repair, round 2).
- Round-1 live gate correctly FALSIFIED the first submission
  (command-role 60 s timeout); recorded as the origin of rev 0.10.

Open observation (owner-ruled non-blocking unless it recurs): near
one clear, Firefox resumed playback; M3 logged no play/toggle
callback and causality could not be reconstructed. Watch for
recurrence in IMP-004/005 live runs.

### AI-IMP-004 — artwork live matrix (PASSED 2026-08-17)

Owner-verified with the M4 dev bundle (real library + generated
fixtures), normal competing sessions:

- Fixture and real-library covers appear within the 3 s bound;
  metadata leads, art follows; elapsed persists after the art phase.
- Art-less track shows no stale art; no late art from departed
  tracks on rapid transitions; soak + natural transition survived.
- OWNER OBSERVATION → ruled rev 0.12: visible art drop/reload on
  every publish — pause/play flickers (souvlaki set_metadata
  rebuilds the dict and re-queues the async image load even for an
  identical URL) and same-album skips drop art (our ruled
  art-less-first policy). Owner overrules the default: HOLD art
  through transitions, swap on resolve. Cut as AI-IMP-007.
- M4 bundle left RUNNING at the owner's pleasure (daily use) until
  IMP-006 ships the real installer; teardown deferred by owner.

## 1.0 BLOCKERS — deferred mandatory gates (owner ruling, 2026-08-19)

The owner ruled development moves on without hard-QA passes of the
full matrices below; they REMAIN mandatory and must be run (in full,
against the then-current build) before any 1.0 tag. Nothing ships as
1.0 on the strength of the provisional passes recorded above/below.

### Wave 4 combined live matrix — PROVISIONAL PASS (2026-08-19)

Owner-verified informally on the real installed bundle (main
`1466f43`): install works after the round-4 argv repair, card shows
metadata/art/icon (no white badge), "all the features work" incl.
remote commands and artwork continuity in normal use; M4 dev bundle
retired in-session (process killed, bundle removed). Round-4 origin:
step 1 FALSIFIED the first install live — plutil array-index quirk
left a two-argument plist and a throttled crash loop; fixed and
regression-tested in `1466f43`.

NOT yet exercised rigorously (the pre-1.0 re-run below covers them):
idempotent re-install / changed-update reload count, rapid-skip
races, mpc clear, kill/restart MPD + >60 s soak, SIGTERM no-relaunch,
logout/login persistence, double uninstall.

### Pre-1.0 full re-run (the blocker)

Run the complete Wave 4 matrix below, unmodified, plus any matrix
rows added by later waves, against the release-candidate build. This
re-run happens regardless of interim passes — it would be required
at 1.0 even if today's run had been exhaustive.

## Wave 4 combined live matrix (reference protocol)

Protocol preserved for the pre-1.0 re-run (originally the Wave 4
close gate; provisionally passed 2026-08-19, see above). This session also retires
the running M4 dev bundle (its teardown is step 1). Preconditions:
normal competing Now Playing sessions are fine (equivalence ruling,
IMP-001); MPD running with a mixed queue including same-album
neighbors, a different-album track, and at least one no-art track.

1. Install: `./install.sh` from main; confirm the M4 dev bundle is
   retired in the same session, the card shows the real icon (no white
   placeholder dot), metadata/art appear, and all five remote commands
   act on MPD.
2. Idempotence: re-run `./install.sh` unchanged — confirm the daemon
   PID is NOT restarted and the script reports no changes. Then apply
   a changed update (any rebuild) and confirm exactly one controlled
   reload.
3. Continuity (§5.3): pause/play with art — no art flicker. Move to a
   same-album track, then a different-album track — no intermediate
   art-less card; replacement swaps only when ready. Move to a no-art
   track — old art lingers briefly, then the card goes art-less.
4. Races: change tracks rapidly during artwork fetch — no
   departed-track art may publish. `mpc clear` — card disappears; start
   a song — clean republish.
5. Lifecycle (§5.5): kill MPD — card clears before retry begins.
   Restart MPD — card returns unaided (no nowplayd restart), cached
   art revalidated. Hold a >60 s quiet soak while connected.
6. Shutdown: SIGTERM the daemon — acknowledged clear, exit 0, and NO
   launchd relaunch (SuccessfulExit=false). Logout/login — agent
   returns on its own.
7. Uninstall: `./uninstall.sh` — bundle, plist, agent, cache, and Now
   Playing entry all gone; config preserved (log is preserved too, by
   design). Run it again — clean no-op.

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
