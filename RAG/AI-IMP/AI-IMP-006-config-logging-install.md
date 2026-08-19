---
node_id: AI-IMP-006
tags:
  - IMP-LIST
  - Implementation
  - packaging
  - config
kanban_status: completed
depends_on: [AI-IMP-001, AI-IMP-003, AI-IMP-004, AI-IMP-005]
parent_epic: [[AI-EPIC-001-mpd-macos-now-playing-bridge]]
confidence_score: 0.75
date_created: 2026-08-16
date_completed:
---

# AI-IMP-006-config-logging-install

## Summary of Issue #1

Implements SPEC.org §11 FR-6/FR-7/FR-8/FR-9: configuration
(zero-config default, TOML, env overrides), structured logging,
production bundle assembly (LSUIElement, informed by the AI-IMP-001
spike's throwaway scripts), and idempotent install/update with a
documented uninstall. Done-state: `./install.sh` from a clean clone
yields a running launchd agent surviving logout/login; re-run is a
no-op; `./uninstall.sh` removes bundle, agent, and cached artwork;
README documents all of it.

### Out of Scope

- Homebrew formula / notarization / code-signing beyond ad-hoc
  (recorded in SPEC.org §9 as future work by the Review Lead if
  wanted).
- Seek/volume config knobs (§9 defer stands).
- Log rotation (corrected rev 0.4: launchd's
  `StandardOutPath`/`StandardErrorPath` redirect only and provide NO
  rotation — v1 ships documented unrotated file logging per §5.4;
  rotation/unified logging is §9 future work).

### Design/Approach

`src/config.rs`: precedence env > TOML
(`~/.config/nowplayd/config.toml`) > defaults, with the EXACT §5.4
rev 0.13 schema: `NOWPLAYD_MPD_ADDRESS`, `NOWPLAYD_MPD_PASSWORD`,
`NOWPLAYD_CACHE_DIR`, `NOWPLAYD_LOG_LEVEL`; one address syntax
distinguishing TCP from unix socket; missing file = defaults,
malformed = loud failure; redacted Debug on config AND
source-report types. Shipping identity per §5.4 rev 0.13:
`io.github.animegolem.nowplayd` (bundle id, agent label, plist
name), `~/Applications/nowplayd.app`, `~/Library/Caches/nowplayd/`,
`~/Library/Logs/nowplayd.log`. Supervision split: launchd
`KeepAlive = { SuccessfulExit = false; }` + `ThrottleInterval`,
config preflighted before bootstrap. Logging: tracing on stderr in
all modes, `StandardErrorPath` owns the file, immutable
`mpd_protocol=off` applied after user verbosity. Supplies runtime
inputs to the 005 supervisor and swaps ONLY its interim sink. Secrets never
logged (FR-8: log the source of each setting and non-secret values at
startup). A TOML-stored password is a documented plaintext threat
boundary (§11 FR-7 rev 0.4): README states it plainly, `install.sh`
sets/verifies owner-only permissions (0600) on the config file, and
the value is redacted from every log path. Logging via `tracing` with
file target when run under launchd; README documents the unrotated-v1
bound and log locations. `packaging/`: `Info.plist` template, `build-bundle.sh`
(release build → `.app`), `install.sh` (build, copy to
`~/Applications`, render launchd plist to
`~/Library/LaunchAgents/dev.nowplayd.plist`, `launchctl bootstrap` —
each step idempotent via compare-before-write), `uninstall.sh`
(bootout, remove bundle/plist/cache, keep config with a note).

### Files to Touch

- `src/config.rs`, `src/main.rs` (config + tracing init): new/edit.
- `Cargo.toml`: `tracing`, `tracing-subscriber`, TOML parser.
- `packaging/Info.plist`,
  `packaging/io.github.animegolem.nowplayd.plist.tmpl`,
  `packaging/build-bundle.sh`, `packaging/nowplayd.icns` (interim
  fixture-derived asset, provenance §5.4 rev 0.13), `install.sh`,
  `uninstall.sh`: new.
- Root `Cargo.lock`, `src/lib.rs`, config/logging tests: in scope.
- `README.md`: install/update/uninstall/config documentation.
- `RAG/HUMAN-TESTING.md`: install-path live checks.

### Implementation Checklist

<CRITICAL_RULE>
Before marking an item complete on the checklist MUST **stop** and
**think**. Have you validated all aspects are **implemented** and
**tested**?
</CRITICAL_RULE>

- [x] Config precedence env > TOML > default with unit tests per
      layer and a malformed-TOML loud-failure test.
- [x] Startup log: each setting's value (secrets redacted) and its
      source (FR-8); asserted in a capture test.
- [x] `tracing` wired through mpd/platform/lifecycle modules; file
      target under launchd, stderr otherwise.
- [x] Credential fence (§11 FR-7 rev 0.7): the subscriber PERMANENTLY
      disables the `mpd_protocol` tracing target — no env verbosity
      may re-enable it; capture test proves a sentinel password never
      appears at the most verbose permitted configuration; app logs
      record auth success/failure only.
- [x] `build-bundle.sh` produces a valid LSUIElement `.app` from
      `cargo build --release`, INCLUDING an app icon
      (`CFBundleIconFile` + `Resources/*.icns`) — macOS badges the
      Now Playing card with the source app's icon, and an iconless
      bundle renders a white placeholder dot (owner-observed on the
      M4 dev bundle, 2026-08-17). Icon asset choice is the owner's;
      the dev-bundle icns (fixture-art derived) is the interim.
- [x] `install.sh` idempotent: second run changes nothing and says
      so; update path (newer binary) replaces bundle and reloads
      agent.
- [x] `uninstall.sh` removes bundle, agent, artwork cache; leaves
      config with printed notice; second run is a clean no-op.
- [x] Config file permissions: `install.sh` enforces 0600 when a
      password is present; README documents the plaintext boundary.
- [x] README: quick start, config reference (incl. password threat
      boundary), update, uninstall, troubleshooting (log locations,
      unrotated-log note).
- [x] Live checks PROPOSED IN THE SUBMISSION (HUMAN-TESTING is the
      Review Lead's): clean install with icon (no white-dot badge);
      second install no-op without restart; changed update reloads
      once; logout/login persistence; uninstall + second-uninstall
      no-op; clean signal exit does not relaunch under
      SuccessfulExit=false.
- [x] Installer safety per §5.4 rev 0.13: refuse unsafe/empty HOME,
      validate exact targets before recursive removal, staged
      replacement with bounded unload wait; shell-level temp-root
      tests where possible.
- [x] Gates green; counts reported.

### Acceptance Criteria

**Scenario:** Clean-machine lifecycle (§7).
**GIVEN** a clean clone on a mac with mpd running.
**WHEN** `./install.sh` runs.
**THEN** the agent is loaded, Control Center works, and re-running
`install.sh` reports no changes.
**WHEN** the owner logs out and back in.
**THEN** the daemon is running without intervention.
**WHEN** `./uninstall.sh` runs.
**THEN** bundle, agent, and cache are gone; no Now Playing entry
remains; config file is preserved with a notice.

### Issues Encountered

<!--
The comments under the 'Issues Encountered' heading are the only
comments you MUST not remove
This section is filled out post work as you fill out the checklists.
You SHOULD document any issues encountered and resolved during the sprint.
You MUST document any failed implementations, blockers or missing tests.
-->

- Implemented the exact rev-0.13 identity throughout. The older
  `dev.nowplayd.plist` example in Design/Approach is superseded by the same
  section's explicit `io.github.animegolem.nowplayd` ruling and by SPEC.org
  rev 0.13.
- Imported the owner-authorized M4 development icon without modifying or
  stopping the running M4 bundle. SHA-256:
  `d9b49fdadef6055882f8d8f1dace9d79dac463d5ff2f4d9b8ed876af53e0d111`.
- Automated gate: 71 tests passed (37 library, 8 binary, 9 artwork, 1
  packaging safety, 15 protocol, 1 isolated-MPD smoke), with `cargo build`
  and `cargo clippy -- -D warnings` green. Linux cross-check passed for
  `x86_64-unknown-linux-gnu`.
- Packaging checks passed: shell syntax, both plist lints, ad-hoc code-sign
  verification, config preflight, unsafe-HOME target fence, second-install
  no-op, changed-update single reload, password-file mode 0600, uninstall,
  and second-uninstall no-op.
- The macOS shipping/live matrix is proposed in the submission rather than
  recorded here; `RAG/HUMAN-TESTING.md` remains untouched for the Review Lead
  and owner.


## Closure (2026-08-19)

Merged to main (`0253e49` wave tip; installer argv repair `1466f43`).
Owner ruled a PROVISIONAL live pass sufficient to close during
development; the full Wave 4 combined matrix is a mandatory pre-1.0
blocker recorded in RAG/HUMAN-TESTING.md.
