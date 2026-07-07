# Hardwave DAW

The first DAW built for harder styles. Tauri 2 desktop app: React 18 frontend
over a Rust audio engine.

## Architecture

```
packages/daw-ui/     React 18 + Vite + zustand frontend (the whole UI)
src-tauri/           Tauri shell: command handlers, diagnostics, updater
crates/
  hardwave-engine    audio graph, transport, undo history, offline render
  hardwave-dsp       DSP primitives (filters, dynamics, resampling, recording)
  hardwave-audio-io  cpal device management, input monitoring
  hardwave-midi      MIDI events, generators (arp/strum), mappings
  hardwave-plugin-host  VST3/CLAP scanning + hosting, GUI scaling
  hardwave-native-plugins  bundled instruments/FX as HostedPlugin impls
  hardwave-project   .hwp project model (MessagePack+zstd, atomic saves)
  hardwave-metering  meters shared UI<->engine
```

Data flow: UI calls Tauri commands (`src-tauri/src/commands/`) → engine
mutations go through channel-based command queues → the audio callback
(`hardwave-engine/src/engine.rs`, `EngineCallback::process`) never blocks on
UI locks (`try_lock` + defer; see `rt_safety_tests`).

Frontend state lives in zustand stores (`packages/daw-ui/src/stores/`).
Failure-visible IPC goes through `src/api/invoke.ts` (error toasts + Retry).

## Development

```bash
# Prereqs: Rust stable, Node 20, and on Linux the Tauri system deps
# (libasound2-dev libjack-jackd2-dev libgtk-3-dev libsoup-3.0-dev
#  libjavascriptcoregtk-4.1-dev).

cd packages/daw-ui && npm ci          # frontend deps
npm run dev                           # Vite dev server (UI only, mock Tauri)
cargo tauri dev                       # full app (from repo root)

npm run screenshot                    # headless PNGs of key views → /tmp
```

## Testing

```bash
./scripts/lint.sh          # fmt + frontend typecheck + clippy -D warnings
cargo test --workspace     # full Rust gate (what release.sh runs)
npm run test:unit          # Vitest store tests (packages/daw-ui)
npm test                   # Playwright UI specs (needs npx playwright install chromium)
```

## Releasing

```bash
./scripts/release.sh <patch|minor|major> "commit message with - fix:/feat: bullets"
```

The script typechecks + unit-tests + builds the frontend, runs
`cargo test --workspace`, bumps `tauri.conf.json` + workspace `Cargo.toml` in
lockstep, generates the changelog from commit bullets, then commits, pushes and
tags `v*`. The tag fires `.github/workflows/release.yml`: fmt + clippy
(hard error) + tests, then Windows/macOS/Linux builds, the updater feed
(`latest.json`), and the Discord changelog post. Users update in-app via
Help → Check for updates.

Bullet discipline matters: `- fix:` / `- feat:` / `- improve:` lines in the
commit body become the user-facing changelog; a release with no bullets is
refused.

## Diagnostics

Every session logs to `~/.hardwave-daw/logs/` (last 5 kept). Panics append a
backtrace and raise an in-app banner. Users attach logs via
Help → Export diagnostics.
