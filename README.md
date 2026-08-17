# nowplayd

nowplayd is a small MPD peer daemon that publishes the current song to macOS
Now Playing and translates media keys, Control Center, lock-screen, and
headphone commands back to MPD. It works independently of the frontend you use.

## Quick start (macOS)

Requirements: Rust, a running MPD server, and macOS 13 or newer.

```sh
git clone https://github.com/animegolem/nowplayd.git
cd nowplayd
./install.sh
```

The installer builds a release `LSUIElement` bundle at
`~/Applications/nowplayd.app`, preflights the effective configuration, and
loads `io.github.animegolem.nowplayd` as a per-user launchd agent. Running it a
second time is a no-op when the installed bundle and plist are unchanged.

The in-repo icon is the owner-authorized M4 development icon generated from
`spike/fixture.jpg`; its SHA-256 is
`d9b49fdadef6055882f8d8f1dace9d79dac463d5ff2f4d9b8ed876af53e0d111`.

## Configuration

The optional file is `~/.config/nowplayd/config.toml`. Environment variables
override TOML, and TOML overrides defaults.

```toml
mpd_address = "tcp://localhost:6600" # or unix:///absolute/path/to/socket
mpd_password = "secret"
cache_dir = "/Users/you/Library/Caches/nowplayd"
log_level = "info" # trace, debug, info, warn, error
```

| TOML key | Environment override | Default |
|---|---|---|
| `mpd_address` | `NOWPLAYD_MPD_ADDRESS` | `tcp://localhost:6600` |
| `mpd_password` | `NOWPLAYD_MPD_PASSWORD` | unset |
| `cache_dir` | `NOWPLAYD_CACHE_DIR` | `~/Library/Caches/nowplayd` |
| `log_level` | `NOWPLAYD_LOG_LEVEL` | `info` |

Addresses deliberately require a scheme so TCP and Unix sockets cannot be
confused. Cache paths must be absolute. A missing config file is normal; a
present malformed file stops startup loudly. Check without starting the daemon:

```sh
~/Applications/nowplayd.app/Contents/MacOS/nowplayd --check-config
```

Passwords in TOML are plaintext at rest. `install.sh` enforces mode `0600` when
the file contains `mpd_password`, but anyone with access to your user account
can still read it. Prefer `NOWPLAYD_MPD_PASSWORD` when your launch environment
can provide it securely. Password values are redacted from application logs,
and the credential-bearing `mpd_protocol` tracing target is permanently off
even at `trace` verbosity.

## Updating

Pull the new source and run `./install.sh` again. A changed bundle or plist is
installed by booting out the old agent, waiting up to five seconds for shutdown,
replacing from a completed staging directory, and bootstrapping once. The
daemon clears Now Playing before its clean signal exit.

## Uninstalling

```sh
./uninstall.sh
```

This boots out the agent and removes the app, launchd plist, and artwork cache.
It deliberately preserves `~/.config/nowplayd/config.toml` and reports that
choice. A second uninstall is a clean no-op.

## Troubleshooting

- Log: `~/Library/Logs/nowplayd.log`
- Agent: `launchctl print gui/$(id -u)/io.github.animegolem.nowplayd`
- Configuration preflight: `./target/release/nowplayd --check-config`
- Reinstall/update: `./install.sh`

The v1 log is intentionally unrotated: launchd redirects stderr to the file but
does not rotate it. Remove or archive it manually if it grows too large.
nowplayd reconnects to MPD indefinitely with capped jittered backoff; MPD
outages do not require restarting the launchd agent.

The normative behavior is in [SPEC.org](SPEC.org), with implementation history
in [RAG/](RAG/INDEX.md).
