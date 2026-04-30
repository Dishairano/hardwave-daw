# Licensing

Hardwave DAW uses a split license model. Different parts of this repo are governed by different licenses depending on what they are.

## TL;DR

| What | License | File |
|---|---|---|
| The DAW application (shell, engine, UI, host, native plugins) | **GPL-3.0-or-later** | [`LICENSE`](LICENSE) |
| The Plugin SDK (public API for third-party plugin developers) | **Apache-2.0** | [`LICENSE-SDK`](LICENSE-SDK) |

If you are **using** Hardwave DAW: it's free, forever. GPL.

If you are **building a plugin** for Hardwave DAW: you can ship under any license you want — commercial, closed-source, GPL, MIT, whatever. The Apache-2.0 SDK does not infect your plugin.

If you are **forking** Hardwave DAW itself: your fork must remain GPL-3.0-or-later. You can sell it, you can rebrand it, but you must publish source for any changes you distribute.

## Why the split

This is the same pattern used by Wine, Blender, and OBS.

- **The shell is GPL-3.0** because we want the DAW itself — and all forks of it — to stay open. If someone builds a closed-source proprietary fork of our work, takes our momentum, and walks away with it, the project dies. GPL prevents that.
- **The SDK is Apache-2.0** because plugin developers are not us. They are independent businesses. They need to be free to ship closed-source commercial plugins on their own terms without their code being pulled into a copyleft license. Apache-2.0 also includes an explicit patent grant, which matters when commercial plugin vendors evaluate whether building on our SDK is safe.

A copyleft host with a permissive plugin SDK is a deliberately-chosen, well-established pattern. It maximizes openness of the platform while maximizing the ecosystem of commercial plugins that can build on top.

## What counts as "the SDK"

The SDK is the set of crates and headers that third-party plugin developers link against to build a plugin compatible with Hardwave DAW. As of this writing, that crate has not yet been split out — when it is, it will live under `crates/hardwave-plugin-sdk/` (or similar) and its `Cargo.toml` will declare `license = "Apache-2.0"`.

Until that split happens, treat everything in this repo as GPL-3.0-or-later.

## Plugin compatibility

Hardwave DAW also hosts standard **VST3** and **CLAP** plugins. Those formats have their own licensing terms, set by Steinberg and the CLAP authors respectively, independent of Hardwave's licenses. Plugins built against VST3 or CLAP are unaffected by anything in this repo.

## Contributor License Agreement

There is no CLA. Contributions to the GPL portion are accepted under GPL-3.0-or-later; contributions to the SDK portion (when it exists) are accepted under Apache-2.0. By submitting a pull request, you agree your contribution is licensed under the same terms as the file or directory you are modifying.

## Questions

Open an issue tagged `licensing` or ask in Discord.
