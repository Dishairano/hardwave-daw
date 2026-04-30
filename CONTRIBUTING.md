# Contributing to Hardwave DAW

Thank you for considering a contribution. This guide is short on purpose — we'd rather you spend time on the code than on reading rules.

## Before you start

1. **Read [`LICENSING.md`](LICENSING.md).** By submitting a PR you agree your contribution is licensed under the same terms as the file you are modifying (GPL-3.0-or-later for the DAW shell, Apache-2.0 for the SDK once it exists).
2. **Read [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md).**
3. **Check existing issues and pull requests** so you don't duplicate work in flight.

There is no Contributor License Agreement.

## Where to start

If this is your first contribution, browse [`good first issue`](https://github.com/Dishairano/hardwave-daw/issues?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22). Those are scoped, low-context, and reviewed faster than larger changes.

If you have something bigger in mind, **open an issue first** to align on direction before writing code. Big PRs that arrive without prior discussion are not necessarily rejected, but they are slower to review and more likely to need rework.

## Development setup

Prerequisites:

- Rust (stable, latest)
- Node.js 20+
- Platform-specific Tauri deps: [tauri.app/start/prerequisites](https://tauri.app/start/prerequisites/)

```bash
git clone https://github.com/Dishairano/hardwave-daw.git
cd hardwave-daw
cd packages/daw-ui && npm install && cd ../..
cargo tauri dev
```

If `cargo tauri dev` fails on a fresh clone, the most common cause is a missing system dep (libwebkit2gtk on Linux, Xcode CLT on macOS, WebView2 on Windows). The Tauri prerequisites page covers all of them.

## Branch & PR flow

1. Fork the repo (or create a branch if you have push access).
2. Branch from `master`. Branch names are not enforced; pick something descriptive.
3. Make your change.
4. Run the checks listed below.
5. Open a PR against `master`. Fill in the PR description: what changed, why, and how you tested it.
6. A maintainer will review. Expect feedback — most PRs need at least one round of changes.

We squash-merge by default. Your individual commits don't need perfect messages; the squash commit will be cleaned up at merge.

## Required checks

Before opening a PR, your branch must pass:

```bash
# Rust
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# Frontend
cd packages/daw-ui
npm run typecheck
npm run lint
npm test
```

CI runs the same checks on every PR. If CI fails, fix it before requesting review.

## Style

- **Rust** — `cargo fmt` is the source of truth. Don't argue with it.
- **TypeScript** — eslint config in `packages/daw-ui/`. Don't disable rules in your PR; if a rule is wrong, raise it as a separate discussion.
- **Naming** — match what's already in the file. Consistency beats personal preference.
- **Comments** — write them only when the *why* is non-obvious. Don't comment what the code already says.

## Audio code rules

If you are touching the audio thread (anything in `crates/hardwave-engine`, `crates/hardwave-audio-io`, or the realtime path of the plugin host):

- **No allocations.** Use the existing lock-free primitives (`rtrb`, `crossbeam-channel`).
- **No locks.** No `Mutex`, no `RwLock`, no `parking_lot::Mutex`. Period.
- **No I/O.** No file reads, no logging from realtime threads. Send to a non-realtime worker via channel.
- **No panics.** A panic in the audio thread is a crash. Use `Result` and recover.

Reviewers will block any PR that breaks these rules in the realtime path.

## Reporting bugs

Open an issue. Include:

- What you did (steps).
- What you expected.
- What happened instead.
- OS, version (`Hardwave DAW > About`), and audio driver/device.
- Any logs (`~/Library/Logs/com.hardwave.daw/` on macOS, equivalent on other platforms).

A short minimal repro project beats a long prose description every time.

## Asking questions

- **Bug or proposed change** — open an issue.
- **General question or "how do I…"** — GitHub Discussions.
- **Real-time chat** — Discord (link will be added once public server opens).

## What we will and will not merge

We will merge:

- Bug fixes with a clear repro.
- Performance improvements with before/after measurements.
- New features that have been discussed in an issue first.
- Documentation, tests, refactors that pay for themselves.

We will not merge:

- Code that adds locks, allocations, or I/O on the audio thread.
- "Style cleanup" PRs that touch hundreds of files for no behavioral reason.
- AI-generated changes that the author has not personally read, understood, and tested.
- Changes that break the public licensing model (see [`LICENSING.md`](LICENSING.md)).

## Recognition

Every contributor whose PR is merged will be listed in our quarterly contributor digest. Your username, your work, your link. We don't take credit for what you build.

Welcome aboard.
