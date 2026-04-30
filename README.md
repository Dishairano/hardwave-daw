# Hardwave DAW

**A free, open-source digital audio workstation with real-time multiplayer.** Built in Rust + Tauri. Hosts your existing VST3 and CLAP plugins. One Hardwave account links every plugin you own.

> Status: pre-release. Currently shipping `0.x` builds. Plugin hosting, multiplayer, and the public Plugin SDK are landing across the next few months.

---

## Why another DAW?

Every existing DAW is one of two things: a $200–$700 closed-source product owned by a company that can deprecate your projects when it suits them, or an old open-source project with a UX from a decade ago. Both shapes leave the same gap unfilled.

Hardwave DAW fills it. Three commitments:

1. **Free, forever.** No price tag, no upsell tier, no "Pro" version. The DAW itself is GPL-3.0. Forks are allowed; closed-source forks are not. We make money on plugins, not on the host.
2. **Real-time multiplayer is native.** Two people opening the same project on different machines edit it at the same time. Not file-based "collaboration" — actual co-editing, like Figma or a Google Doc, with the same conflict-free CRDT model underneath.
3. **One account, every plugin.** Sign in with Hardwave once and every Hardwave plugin you own activates automatically across every machine. We're also building the SDK so any plugin developer can plug into the same flow.

If those three commitments matter to you, you're who this is for.

## Quickstart

### Download a release

Pre-built installers for macOS and Windows are attached to each [GitHub release](https://github.com/Dishairano/hardwave-daw/releases). Linux builds are not yet shipped; build from source for now.

### Build from source

You will need Rust (stable), Node.js 20+, and the platform-specific Tauri prerequisites: see [tauri.app/start/prerequisites](https://tauri.app/start/prerequisites/).

```bash
git clone https://github.com/Dishairano/hardwave-daw.git
cd hardwave-daw
cd packages/daw-ui && npm install && cd ../..
cargo tauri dev
```

For a production build:

```bash
cd packages/daw-ui && npm run build && cd ../..
cargo tauri build
```

The built binary lands in `src-tauri/target/release/bundle/`.

## Architecture

| Layer | Tech | Crate / Path |
|---|---|---|
| Audio engine | Rust, lock-free | `crates/hardwave-engine` |
| Audio I/O | CPAL | `crates/hardwave-audio-io` |
| MIDI I/O | midir | `crates/hardwave-midi` |
| Plugin host | VST3 + CLAP | `crates/hardwave-plugin-host` |
| Project format | Versioned, zstd-compressed | `crates/hardwave-project` |
| DSP primitives | rustfft, rubato | `crates/hardwave-dsp` |
| Native plugins | Built-in instruments/effects | `crates/hardwave-native-plugins` |
| Metering | Lock-free RMS/peak | `crates/hardwave-metering` |
| Desktop shell | Tauri 2 | `src-tauri/` |
| UI | React 18 + TypeScript + Vite | `packages/daw-ui/` |

Real-time multiplayer is implemented as a CRDT layer on top of the project format. Documentation will land alongside the first multiplayer release.

## Plugin SDK

If you build plugins, the public SDK lets you target Hardwave DAW directly with full access to project state, transport, and the Hardwave account session. The SDK is licensed under **Apache-2.0** so you can ship closed-source commercial plugins on your own terms.

The SDK crate is being split out from the existing `hardwave-plugin-host` host code and is not yet stable. Watch the repo or join Discord to hear when the first SDK release ships.

## Contributing

We want contributors. The project's first 100 contributors will be the people who decide what kind of community this becomes — that's a real opportunity if you want to shape a tool you'll use every day.

- Read [`CONTRIBUTING.md`](CONTRIBUTING.md) before opening a PR.
- Browse [issues tagged `good first issue`](https://github.com/Dishairano/hardwave-daw/issues?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22) for places to start.
- Behavior is governed by [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md).

## Community

- **GitHub Discussions** for questions, ideas, and feature requests.
- **Discord** — invite link will be posted at [hardwavestudios.com](https://hardwavestudios.com) and pinned on this README once the public server opens.
- **Strategy doc** — the public-facing strategy and roadmap rationale lives at [suite.hardwavestudios.com/daw-strategy/](https://suite.hardwavestudios.com/daw-strategy/).

## License

Split license. See [`LICENSING.md`](LICENSING.md) for the full explanation.

- The **DAW application** (this repo, except the SDK) is licensed under **GPL-3.0-or-later** — see [`LICENSE`](LICENSE).
- The **Plugin SDK** (when split out) is licensed under **Apache-2.0** — see [`LICENSE-SDK`](LICENSE-SDK).

You are free to use, modify, and distribute the DAW under those terms. You are free to ship closed-source commercial plugins built against the SDK.

---

Hardwave Studios — Hasselt, Belgium.
