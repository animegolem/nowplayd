---
node_id: AI-IMP-006
tags:
  - IMP-LIST
  - Implementation
  - packaging
  - config
kanban_status: planned
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

`src/config.rs`: precedence env (`NOWPLAYD_*`) > TOML
(`~/.config/nowplayd/config.toml`) > defaults; keys: mpd address
(tcp/unix), password, cache dir, log level/target. Secrets never
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
- `packaging/Info.plist`, `packaging/dev.nowplayd.plist.tmpl`,
  `packaging/build-bundle.sh`, `install.sh`, `uninstall.sh`: new.
- `README.md`: install/update/uninstall/config documentation.
- `RAG/HUMAN-TESTING.md`: install-path live checks.

### Implementation Checklist

<CRITICAL_RULE>
Before marking an item complete on the checklist MUST **stop** and
**think**. Have you validated all aspects are **implemented** and
**tested**?
</CRITICAL_RULE>

- [ ] Config precedence env > TOML > default with unit tests per
      layer and a malformed-TOML loud-failure test.
- [ ] Startup log: each setting's value (secrets redacted) and its
      source (FR-8); asserted in a capture test.
- [ ] `tracing` wired through mpd/platform/lifecycle modules; file
      target under launchd, stderr otherwise.
- [ ] Credential fence (§11 FR-7 rev 0.7): the subscriber PERMANENTLY
      disables the `mpd_protocol` tracing target — no env verbosity
      may re-enable it; capture test proves a sentinel password never
      appears at the most verbose permitted configuration; app logs
      record auth success/failure only.
- [ ] `build-bundle.sh` produces a valid LSUIElement `.app` from
      `cargo build --release`, INCLUDING an app icon
      (`CFBundleIconFile` + `Resources/*.icns`) — macOS badges the
      Now Playing card with the source app's icon, and an iconless
      bundle renders a white placeholder dot (owner-observed on the
      M4 dev bundle, 2026-08-17). Icon asset choice is the owner's;
      the dev-bundle icns (fixture-art derived) is the interim.
- [ ] `install.sh` idempotent: second run changes nothing and says
      so; update path (newer binary) replaces bundle and reloads
      agent.
- [ ] `uninstall.sh` removes bundle, agent, artwork cache; leaves
      config with printed notice; second run is a clean no-op.
- [ ] Config file permissions: `install.sh` enforces 0600 when a
      password is present; README documents the plaintext boundary.
- [ ] README: quick start, config reference (incl. password threat
      boundary), update, uninstall, troubleshooting (log locations,
      unrotated-log note).
- [ ] Live checks appended to `RAG/HUMAN-TESTING.md`: fresh install →
      working media keys; logout/login persistence; uninstall leaves
      no Now Playing entry and no agent.
- [ ] Gates green; counts reported.

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
