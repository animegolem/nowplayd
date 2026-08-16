# nowplayd

MPD → macOS Now Playing bridge.

A standalone daemon that connects to [MPD](https://www.musicpd.org/) as a peer
client and registers with the macOS media layer
(`MPNowPlayingInfoCenter` / `MPRemoteCommandCenter`), so media keys, Control
Center, the lock screen, and AirPods gestures control MPD — no matter which
frontend you use (rmpc, ncmpcpp, euphonica, ...).

Terminal MPD frontends structurally can't register with `mediaremoted`; MPD
holds all playback state anyway, so a small peer daemon closes the gap for
every frontend at once.

## Status

Pre-implementation. The normative spec is [SPEC.org](SPEC.org); work is tracked
in [RAG/](RAG/INDEX.md).

## Planned shape

- Rust, [`souvlaki`](https://crates.io/crates/souvlaki) for the platform layer
  (MPRIS on Linux comes free)
- `idle`-loop MPD client, album art via `albumart`/`readpicture`
- `LSUIElement` app bundle + launchd user agent
