# CI/CD + Dead-Code Cleanup — Design Spec

**Date:** 2026-07-06 · **Requested by founder:** "We really need to clean up the ci/cd's etc etc"
(2026-07-05; execution deferred until the staff-engineer discipline was installed).

## Problem

Accumulated cruft wastes CI minutes and misleads future work:
1. **`dev-build.yml` runs a FULL 3-platform Tauri build on every push** — near-pure
   duplication: every feature is released via a `v*` tag minutes later (full Release matrix),
   and `fast-windows.yml` covers "quick Windows exe".
2. **`frontend-publish.yml` (429 lines) is dead** — fires on `fe-v*` tags which `release.sh`
   can never produce since the hot-swap was retired (v0.200.0) and FRONTEND_ONLY is forced 0.
3. **`ci.yml` never fires** — triggers only on `master`, dormant since May; duplicates dev-build.
4. **`release.sh` carries ~100 lines of dead fe-v*/FRONTEND_ONLY detection** (always forced 0).
5. **`transport/Toolbar.tsx` (~900 lines) is a dead component** — imported by App.tsx, never
   rendered. It already cost 6 wasted releases (the wrong-component trap).
6. **Retired hot-swap machinery**: `register_uri_scheme_protocol(hardwave-app)` in lib.rs +
   the cache/manifest/bundle-download code in `frontend_updater.rs` under `#![allow(dead_code)]`.
   The live updater is only `frontend_update_check_and_apply` + UpdateStatus events.
7. **Stray `fast-v1..v4` tags** on origin (debug triggers, not releases).

## Plan (ordered so the first push already stops the CI burn)

| Step | Change | Risk / gate |
|---|---|---|
| 1 | `dev-build.yml` → `workflow_dispatch` only | None (manual trigger retained) |
| 2 | Delete `frontend-publish.yml`, `ci.yml` | None (dead) |
| 3 | `release.sh`: strip fe-v*/FRONTEND_ONLY machinery; always full release | Re-read script; next release validates |
| 4 | Delete `transport/Toolbar.tsx` + App.tsx import | `npm run build` (tsc) |
| 5 | lib.rs: drop `register_uri_scheme_protocol`; `frontend_updater.rs`: remove dead hot-swap code, KEEP `frontend_update_check_and_apply` + `UpdateStatus` events + anything the splash consumes; drop `#![allow(dead_code)]` | `cargo check` + `clippy --workspace -D warnings` + fmt; grep splash listeners to confirm the kept surface |
| 6 | Delete `fast-v1..4` remote tags; fast-windows.yml deletes its trigger tag after publish | None (fast-build prerelease + workflow stay) |
| 7 | Push as chore commits (no release tag; rides the next release) | functional-smoke on push |

**Explicitly NOT in scope:** the detach diagnostics (windows.rs beacons + web01 collector +
nginx location) — stay until the founder confirms window dragging on v0.204.7, then a
follow-up strips them. Also not in scope: `fl-vst-webviews` repo, server nginx `.bak` files.

## Verification
- Full local gate before push: `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`,
  `cargo test -p hardwave-engine` (fast representative suite), `npm run build` (daw-ui),
  screenshot-harness playlist still (UI unaffected).
- After push: functional-smoke green; confirm NO Dev Build run starts on the push.
- Updater surface check: grep `frontend-update-status` consumers in daw-ui and confirm every
  event variant emitted by the kept Rust path still exists.
