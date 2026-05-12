# Hardwave DAW — Performance Audit

**Date:** 2026-05-12
**Branch:** `redesign-port-from-master`
**HEAD:** `dd7d481`
**Auditor:** Performance Benchmarker subagent — static read of source tree, no profilers run

---

## Executive summary

Three top-of-funnel costs dominate this codebase today, in this order:

1. **`AudioGraph::process` allocates 5+ vecs per node per block on the real-time audio thread** (`crates/hardwave-engine/src/graph.rs:297-352`). With `DEFAULT_INSERT_COUNT = 500` (`crates/hardwave-project/src/project.rs:58`), every block performs ~3000+ `Vec` allocations + an `inputs: Vec<Vec<f32>>` rebuild that scales O(edges×nodes) per block. At 48 kHz / 256-sample buffers that's ~187 blocks/sec — sustained malloc traffic on the audio thread.
2. **The shipped mixer is the legacy `MixerPanelLegacy`, not the v2 you optimized.** `useNewMixer` defaults to `false` (`packages/daw-ui/src/stores/mixerSettingsStore.ts:39`), and `MixerPanel.tsx:18-23` routes most users to `MixerPanelLegacy` which destructures bare `useTrackStore()` at line 30 and renders all 500 strips with no virtualization. None of the Phase 4 wins reach the default user.
3. **Every TrackNode runs the full DSP chain regardless of activity.** `track_node.rs:464` early-exits on `self.muted || !ctx.playing`, but does not skip on "no clips overlap this block + no automation + no chain" — so 500 idle inserts still execute the per-frame clip-overlap loop, filter, fader, peak/RMS, and store atomics every block.

The Phase 4 work that did land (canvas meters via global rAF, `content-visibility`, virtualized v2 scroller, optimistic-local fader drag) is real and correct, but the user has to opt in to even see it.

## Methodology

Static reading of the Rust workspace (`crates/hardwave-{engine,audio-io,plugin-host,project}`, `src-tauri/src/`) and frontend (`packages/daw-ui/src/`) at HEAD `dd7d481` on `redesign-port-from-master`. Grep-counted IPC call sites and selector patterns, line-counted hot files, traced the audio callback path from `cpal::Stream` → `EngineCallback::process` → `AudioGraph::process` → per-node `process`, and verified each Phase 4 claim against the actual file. **No profilers were run**; numeric estimates are first-principles arithmetic against block size × node count × event rate and should be confirmed with a real trace before betting on them. Where I could not measure (FPS under drag, IPC bandwidth, actual plug-in init time) I say so explicitly in the section below.

## Hotspot inventory

### 1. Per-block allocation storm in `AudioGraph::process`

**Evidence:** `crates/hardwave-engine/src/graph.rs:303-346`:

```rust
let inputs: Vec<Vec<f32>> = {
    let mut ch_bufs: Vec<Vec<f32>> = vec![vec![0.0; self.buffer_size]; NODE_CHANNELS];
    let mut delay_scratch = vec![0.0_f32; self.buffer_size];
    for edge_idx in 0..self.edges.len() {
        ...
        let src_vec: Vec<f32> = match self.buffers.get(edge_source)... { Some(ch) => ch.clone(), ...
```

**Why it costs:** Each iteration of the node loop allocates `ch_bufs` (4 × buffer_size f32s = 4 KB at buffer_size 256), `delay_scratch` (1 KB), and for **every edge whose dest is this node** it `.clone()`s the entire source channel `Vec<f32>` (`graph.rs:322`). Then `let mut outputs = vec![vec![0.0f32; self.buffer_size]; NODE_CHANNELS];` (line 345) and `let input_refs: Vec<&[f32]> = inputs.iter().map(...).collect()` (line 344) allocate two more. **All on the audio thread.** With 500 inserts + master + send edges, that's roughly 5 × (500 + 1) = 2500+ heap touches per audio callback. The line-302 comment hints this was a borrow-checker workaround, not a perf choice.

**Estimated impact:** Likely the single biggest contributor to audio-thread CPU and a glitching risk at low buffer sizes. At 48 kHz / 128 samples (2.7 ms budget) this is plausibly the dominant cost.

**Fix proposal:** Pre-allocate `ch_bufs` and `outputs` once per node into a `Vec<NodeScratch>` parallel to `nodes`; pass `&[&[f32]]` slices that reference `self.buffers[source][port]` directly. The `.clone()` on line 322 can become a `&[f32]` borrow once `edge_delays` is moved into a `RefCell`/`UnsafeCell` pattern or split-borrowed via `split_at_mut`.

**Effort:** ~6h.

### 2. `Project::default()` instantiates 500 audio tracks unconditionally

**Evidence:** `crates/hardwave-project/src/project.rs:58-67`:

```rust
pub const DEFAULT_INSERT_COUNT: usize = 500;
...
for i in 1..=DEFAULT_INSERT_COUNT {
    let id = format!("insert-{:03}", i);
    tracks.push(Track::new_audio(id, format!("Insert {}", i)));
}
```

**Why it costs:** Every new project (and every test) builds a 501-track graph immediately. `rebuild_graph` at `engine.rs:1109-1508` walks all 500 to build TrackNodes, allocates 4 × buffer_size f32 buffers per node (`graph.rs:175`), wires 1000 edges to master, and runs `finalize_pdc` (O(edges × edges)). The audio thread then processes all 500 every block forever, even though only a handful are typically used.

**Estimated impact:** Graph rebuild on new-project is O(N²) over edges; per-block cost scales linearly with N. Idle 500-track project still drives the meter loop, all 500 atomic peak/RMS stores per block, and 500 entries in the `daw:trackMeters` JSON payload below.

**Fix proposal:** Lazily promote an "insert slot" from project metadata into a live TrackNode only when it gets clips, a chain, or sends. Keep the 500 in the project struct (so the UI strips show up) but mark them `disabled_in_graph: true` until first use.

**Effort:** 1-2 days.

### 3. The default-mixer route is the un-optimized legacy panel

**Evidence:** `packages/daw-ui/src/components/mixer/MixerPanel.tsx:19-23` + `stores/mixerSettingsStore.ts:39`:

```tsx
const useNew = useMixerSettingsStore((s) => s.useNewMixer)
if (useNew) { return <MixerPanelV2 /> }
return <MixerPanelLegacy />
```

`useNewMixer: initial.useNewMixer ?? false` — false by default.

**Why it costs:** `MixerPanelLegacy` does `const { tracks, setVolume, ... } = useTrackStore()` at line 30 (bare consumer, re-renders on every store change), `useMeterStore()` at line 31 (subscribes the whole meter map, re-renders on every meter tick — 30 Hz), and `allTracks.map((track, idx) => <Strip ... />)` at line 66 with no virtualization. With 500 inserts that's 500 strip components, each re-rendering every meter tick.

**Estimated impact:** This is the dominant frontend cost users actually experience. Roughly 500 strips × 30 Hz = 15 000 React updates/sec on idle.

**Fix proposal:** Flip `useNewMixer` default to `true` after a smoke pass on the v2 in real projects. Mark the legacy file for deletion in two releases.

**Effort:** 30 minutes (flip default) + ongoing legacy removal.

### 4. v2 "fine-grained" selectors do an O(N) array scan per strip

**Evidence:** `packages/daw-ui/src/stores/trackStore.ts:859-880`. Every selector is `useTrackStore((s) => s.tracks.find((t) => t.id === id)?.X)`:

```ts
export const useTrackVolume = (id: string) =>
  useTrackStore((s) => s.tracks.find((t) => t.id === id)?.volume_db ?? 0)
```

36 such `tracks.find` selectors are defined; `ChannelStrip.tsx:54-62` uses 8 of them per strip.

**Why it costs:** Zustand only re-renders when the selector return value changes (good), but Zustand re-runs every subscribed selector on every store update. After a `fetchTracks()`, that's 8 selectors × 500 strips × 500-element `.find` = 2 million comparisons per fetch.

**Estimated impact:** Each `fetchTracks()` is currently O(N²) selector work. With 500 strips this is single-digit ms per fetch in dev, more under JS heap pressure. Mostly invisible until the user does many edits or the track count grows.

**Fix proposal:** Maintain a `tracksById: Record<string, TrackWithClips>` slice next to `tracks` and rewrite selectors as `(s) => s.tracksById[id]?.volume_db ?? 0`. O(1) per selector.

**Effort:** 2h (mostly mechanical — keep both arrays in sync inside `fetchTracks` and the optimistic mutators).

### 5. Every mutation triggers a full `fetchTracks()` round-trip

**Evidence:** `packages/daw-ui/src/stores/trackStore.ts` — grep found **65 mentions of `fetchTracks`**; every mutator ends with `await get().fetchTracks()`. The mutator round-trip is: `invoke('set_track_volume', ...)` → `invoke('get_tracks')` → `Promise.all` of `invoke('get_track_clips', { trackId })` for **every track**:

```ts
fetchTracks: async () => {
    const trackList = await invoke<TrackInfo[]>('get_tracks')
    const tracks: TrackWithClips[] = await Promise.all(
      trackList.map(async (t) => {
        const clips = await invoke<ClipInfo[]>('get_track_clips', { trackId: t.id })
        return { ...t, clips }
      })
    )
```

**Why it costs:** A single fader commit triggers 1 + 1 + 500 = **502 Tauri IPC round-trips** to refresh state. Tauri 2 IPC isn't free — each round-trip serializes JSON across the WebView boundary, runs `serde_json` both sides, and re-acquires the engine lock.

**Estimated impact:** Best case ~50-100ms hitch per commit on 500-track sessions; worse if the lock is contended with the 33 ms meter tick (`src-tauri/src/lib.rs:418`). Most-invoked commands grep confirms: `set_track_volume` (27), `stop` (44), `remove_track` (38), etc., each followed by a `get_tracks` + 500 `get_track_clips`.

**Fix proposal:** Add a `get_tracks_with_clips` Tauri command that returns the whole tree in one round-trip, or — better — make mutator commands return the *delta* (the changed `TrackInfo` only) and patch the store locally.

**Effort:** 4-6h.

### 6. Per-track meter payload is built as `serde_json::Value` and re-serialized

**Evidence:** `src-tauri/src/lib.rs:430-442`:

```rust
let track_payload: Vec<_> = eng
    .track_meter_snapshots()
    .into_iter()
    .map(|(id, pl, pr, rms, pre_fader)| {
        serde_json::json!({ "id": id, "peakL": pl, ... })
    })
    .collect();
```

**Why it costs:** `serde_json::json!` builds a `Value` enum (a recursive boxed thing) for each of 500 tracks every 33 ms, then `app_handle.emit` serializes it back to a string. The intermediate `Value` is pure waste — a `#[derive(Serialize)] struct TrackMeterPayload` would skip a full alloc pass per track per tick.

**Estimated impact:** 500 × 30 Hz = 15 000 `Value` allocations/sec. Burns renderer event-handler CPU and main-thread bandwidth.

**Fix proposal:** Replace with a `#[derive(Serialize)]` struct and `Vec<TrackMeterPayload>`; same serialized shape, ~3-5× less alloc.

**Effort:** 30 min.

### 7. `parallel_eval` exists but is dead code

**Evidence:** Grep finds `ParallelSchedule` and `ThreadPool` only referenced inside `parallel_eval.rs` itself and `lib.rs:12` (the `pub mod` declaration). Nothing in `graph.rs::process` uses them.

**Why it costs:** It doesn't cost anything *running* — but the API claim that the audio thread schedules parallel branches isn't true today. 500 tracks all run serially on the audio callback thread.

**Estimated impact:** Latent — wiring it up is a separate uplift. Worth flagging because the doc comment at the top of `parallel_eval.rs` reads as if it's already integrated.

**Fix proposal:** Either wire `ParallelSchedule::from_dependencies` into `AudioGraph::rebuild_order` and dispatch each layer with `ThreadPool::run_parallel`, OR delete the module so it stops looking shipped.

**Effort:** 1-2 days to wire (and validate against the audio-thread real-time priority — `thread::spawn` per block at line 162 would itself glitch; needs a long-lived pool with condvar wakeups, not the current ephemeral one).

### 8. Plug-in chains run every block even when the track produced silence

**Evidence:** `track_node.rs:464` early-exits on `self.muted || !ctx.playing` but **not** when the track has zero overlapping clips, no automation, no sends arriving. `track_node.rs:723-729` always calls `self.chain.process(...)` post-utility, which iterates `slots` and for each enabled slot calls the plug-in's `process`. With zero input the plug-in still does its full DSP work.

**Why it costs:** Plug-ins are unaware of input zero-ness and will sample-bang their state machines (LFOs, modulation, internal smoothers) every block. Multiply by 500 tracks × every block × any chain length and even a dozen idle compressors on idle tracks burn measurable CPU.

**Estimated impact:** Workflow-dependent, but plausibly 10-30% of audio CPU in a busy session. I am not sure of the exact share — would need a profiler.

**Fix proposal:** Add a `has_signal_this_block: bool` short-circuit before the chain call: if the post-clip-sum L+R is all zeros AND no inputs arrived AND no automation lane targets this chain, skip `self.chain.process`. Track an "active" flag the plug-in itself can opt out of (some plug-ins need to keep running for tail decay).

**Effort:** 4h, plus careful tail-handling for reverbs/delays.

### 9. `service_snapshot_request` clones `track_id_to_node` on the audio thread

**Evidence:** `engine.rs:966-970`:

```rust
let pairs: Vec<(String, crate::graph::NodeId)> = self
    .track_id_to_node
    .iter()
    .map(|(k, v)| (k.clone(), *v))
    .collect();
```

**Why it costs:** Called from `service_snapshot_request` (`engine.rs:955`) which runs once per block — but `tx.take()` (line 958) gates the body, so the clone only fires when a UI snapshot is pending (save). That's fine. **But** the engine.rs comment at line 949-954 ("get_state is allowed on the audio thread") is dangerous — VST3/CLAP `get_state()` can allocate, lock, or take seconds for some plug-ins. This is a future glitch source. Save during playback = potential dropout.

**Estimated impact:** Latent. Triggered only on save; currently OK in practice but a real-world third-party plug-in will eventually glitch this.

**Fix proposal:** Move `get_state()` to a dedicated worker thread that takes a snapshot of the plug-in instance handle, or document that saving during playback may glitch.

**Effort:** 1 day.

### 10. cpal output buffer fixed at 512 samples; no negotiation around device support

**Evidence:** `crates/hardwave-audio-io/src/lib.rs:198`: `buffer_size: 512`; line 444: `buffer_size: cpal::BufferSize::Fixed(self.buffer_size)`. There's no fallback if the device rejects 512.

**Why it costs:** Stream creation fails on devices that don't support exactly 512 samples in fixed mode; some pro interfaces only support powers-of-two ≥ 1024 in exclusive mode. The error is surfaced to the UI but the user can't iterate quickly.

**Estimated impact:** Setup-time issue only, but it does drive support tickets.

**Fix proposal:** Try `Fixed(self.buffer_size)`, on `InvalidArgument` fall back to `BufferSize::Default`, log the chosen size, push it back to `self.buffer_size`.

**Effort:** 1h.

### 11. Cargo release profile ships with `lto = false, opt-level = 2`

**Evidence:** `Cargo.toml:89-101`:

```toml
[profile.release]
opt-level = 2
lto = false
codegen-units = 16
```

**Why it costs:** Audio DSP is the textbook case where LTO + `opt-level = 3` matter — the biquad inner loop (`track_node.rs:163-181`) and the per-sample resample loop (`track_node.rs:585-644`) live and die by inlining + autovectorization. `codegen-units = 16` further suppresses cross-fn inlining.

**Estimated impact:** Likely 20-40% slower DSP than the `release-final` profile (which exists at line 106 but isn't used by CI per the comment at line 93).

**Fix proposal:** Switch CI to `--profile release-final` for actual user-facing builds. The comment at line 92-93 says this is the plan for ship builds.

**Effort:** 15 min (CI workflow).

### 12. `Inputs.iter().map(|v| v.as_slice()).collect()` per node per block

**Evidence:** `graph.rs:344`. Allocates a `Vec<&[f32]>` solely to satisfy the `AudioNode::process` signature.

**Why it costs:** Same family as #1 — one more per-block alloc.

**Fix proposal:** Stash a reusable `Vec<&[f32]>` in `EngineCallback`; clear and refill each iteration.

**Effort:** 30 min.

### 13. `History::push` snapshots the *entire* `Project` clone-by-value

**Evidence:** `engine.rs:47-52`. Each snapshot is a full `Project` clone — with 500 tracks each holding `clips`, `inserts`, `sends`, `automation_lanes`, etc. Cap is 256 (`engine.rs:29`).

**Why it costs:** Memory: 256 × ~(500 tracks × a few KB metadata) = potentially tens of MB sitting in the undo stack. Each `snapshot_before_mutation` is a deep clone.

**Estimated impact:** Memory-bound rather than CPU-bound; matters more on long sessions with many edits.

**Fix proposal:** Switch to an event-sourced undo (store mutation deltas, not snapshots) or use an `Arc<Project>` + copy-on-write for changed branches.

**Effort:** 2 days (deltas) or 1 day (Arc CoW).

## Already-optimized verification

The Phase 4 work from `v0.160.0` → `fe-v0.161.3` was inspected file-by-file:

- **`@tanstack/react-virtual` in v2 mixer** — VERIFIED. `StripsScroller.tsx:64-70` correctly configured with `horizontal: true`, `overscan: 8`, `estimateSize: () => 64`. Only the visible strips render. **Caveat**: this never reaches users because `useNewMixer` defaults to `false` — see hotspot #3.
- **GPU compositor scroll** — VERIFIED. `StripsScroller.tsx:79-88` applies `translate3d(...)` to `innerRef` and calls `virtualizer.scrollToOffset` to sync visibility. `will-change: transform` and `contain: layout paint` are set on the inner div (line 201-203). Correct.
- **Spring physics** — VERIFIED. `animateScroll` (`StripsScroller.tsx:97-134`) is a critically-damped spring with `k=300, c=32`. Looks fine.
- **Canvas meter via global rAF singleton** — VERIFIED. `services/meterStream.ts` is exactly what was claimed: a `Map`+`Set` of registered canvases painted from a single rAF that reads via `useMeterStore.getState()` (line 94 — no subscription, no React re-render). Includes a "skip paint when fill barely moved" optimization at line 65 (good — 0.4% threshold). **Caveat**: only used by `MeterPair` (v2 mixer primitive). The legacy mixer uses `useTrackMeter(id)` selectors (`meterStore.ts:62`) directly and re-renders every meter tick.
- **`content-visibility: auto` on strips** — VERIFIED. `mixer-v2.css:52-56`. Correct, gated to v2.
- **Fine-grained Zustand selectors** — PARTIAL. The selectors exist (`trackStore.ts:859-880`) and v2 `ChannelStrip` uses them. **But** every selector is `s.tracks.find((t) => t.id === id)`, which is O(N). The "fine-grained" claim is true for re-render scope but false for selector cost — see hotspot #4.
- **Optimistic-local fader/pan drag + commit-on-pointerup** — VERIFIED. `trackStore.ts:383-410` implements `setVolumeLocal` / `commitVolume` / `setPanLocal` / `commitPan`. `ChannelStrip.tsx:66-87` wires them correctly. **Caveat**: `commitVolume` *still* calls `fetchTracks()` (line 400), so the pointerup is followed by a 500-clip-fetch storm — see hotspot #5.
- **Single engine lock per meter tick** — VERIFIED. `lib.rs:423-466`. Comment at line 420-422 ("coalesces four separate engine.lock() calls") matches the code.

## Quick wins (≤ 1 hour each)

- Flip `useNewMixer` default to `true` in `mixerSettingsStore.ts:39`. Single biggest visible win — already-done Phase 4 wiring reaches users.
- Switch CI to `cargo build --profile release-final` per `Cargo.toml:106`. Ship-time DSP speed-up at zero risk to the redesign workflow.
- Replace the `serde_json::json!` block in `src-tauri/src/lib.rs:430-442` with a `#[derive(Serialize)] struct TrackMeterPayload`.
- Pre-allocate the `input_refs: Vec<&[f32]>` at `graph.rs:344` into a field on `AudioGraph` and reuse it.
- Add cpal `BufferSize::Default` fallback in `audio-io/src/lib.rs:444` when the fixed size is rejected.
- Hoist the `let order = self.processing_order.clone();` at `graph.rs:298` — iterate by index over the field directly (the clone is unnecessary because nothing inside the loop mutates the order).

## Bigger investments (1+ day)

- **Eliminate audio-thread allocations in `AudioGraph::process`** (#1). The single highest-leverage Rust change. ~6h once a NodeScratch layout is designed.
- **Promote/demote insert tracks lazily** (#2). 1-2 days; keeps the 500-strip UI but stops the engine from paying for them.
- **One-round-trip track-state IPC** (#5). 4-6h; pair with #4 for compounding wins.
- **`tracksById` map for O(1) selectors** (#4). 2h, mechanical.
- **Active-track gating for plug-in chains** (#8). 4h core + careful tail handling. Pairs naturally with #2.
- **Wire `parallel_eval` or delete it** (#7). 1-2 days to wire properly with a long-lived pool; pool-spawning per block as currently written would *cost* more than it saved.

## What I couldn't measure statically

- Real CPU/% per audio block — `cargo flamegraph` or `tracy-rs` over a 500-track session is the only honest answer for hotspots #1/#2/#8. I claim "biggest contributor" for #1 from first principles, not measurement.
- Frontend FPS during fader drag on the legacy mixer with 500 strips. The legacy code path looks bad but Chromium's React/DOM caching may smooth it more than the static read suggests.
- Plug-in init time for real VST3s — `vst3.rs:520 (activate)` doesn't enforce a budget. I am not sure how long a typical commercial plug-in takes to activate at 48k/512; would need a real plug-in scan + load trace.
- IPC bandwidth at 30 Hz × 500-track meter payload. The JSON is ~50-60 bytes/track × 500 = ~30 KB/tick × 30 Hz = ~900 KB/s. WebView2's IPC pipe handles this fine in theory; in practice JSON parsing on the main thread may be a hidden cost.
- Memory growth across long sessions from `History::push` (#13). Would need a memory profiler running for hours.
- WASAPI exclusive-mode latency under real load — only inspectable on Windows hardware.

## Recommended next 5 changes

1. **Flip `useNewMixer` default to `true`** (`mixerSettingsStore.ts:39`). 15 min. Single biggest visible win. The Phase 4 work you already did is real and correct; flipping this lets it reach users. Verify with one smoke session on a heavy project before merging.
2. **Switch CI builds to `--profile release-final`** (`Cargo.toml:106`). 15 min. The audio DSP loops in `track_node.rs` and `engine.rs` are exactly the kind of code that `lto = "thin", opt-level = 3` exists for. The redesign rationale for `lto = false` (faster CI) made sense during heavy iteration but every shipped build is currently paying the cost.
3. **Kill audio-thread allocations in `AudioGraph::process`** (#1). 6h. Pre-allocate `NodeScratch { inputs: Vec<Vec<f32>>, outputs: Vec<Vec<f32>>, input_refs: Vec<&[f32]> }` once per node at `add_node` time. Inside `process`, `inputs[port].fill(0.0)` instead of `vec![vec![0.0;...];...]`. The `.clone()` at `graph.rs:322` becomes a borrow once `edge_delays` is moved to live alongside the edges as `Option<Box<EdgeDelayLine>>` with split borrows. This is the foundation everything else builds on — once the audio thread is alloc-free, increasing track count is no longer scary.
4. **`tracksById` O(1) selectors + one-shot fetch** (#4 + #5 combined). 6h. Add `tracksById: Record<string, TrackWithClips>` to `trackStore`, populate inside `fetchTracks` and every optimistic mutator. Rewrite all 36 selectors. Replace `fetchTracks` body with a single `get_tracks_with_clips` Tauri command. Compounds: fader-commit goes from ~500 IPCs + 500 × 8 linear scans down to 1 IPC + 8 hash lookups.
5. **Lazy insert promotion** (#2). 1-2 days. Mark `Track::disabled_in_graph` for tracks that have no clips, no chain, no sends, no automation, no monitoring. Skip them in `rebuild_graph` (`engine.rs:1155-1354`) and in `sync_track_meters`. Project still defaults to 500 strips (FL-style UX preserved), but the audio thread only runs the handful actually used. This is the change that turns "supports 500 tracks" from aspirational into honest.

After these five, the worst-case mixer interaction should be a different shape entirely — fader drag without IPC storm, audio thread allocation-free, only-active tracks processed, and the GPU-composited v2 mixer in front of the user. Then re-benchmark with a real profiler before deciding what's next.

---

*Audit performed via static read at HEAD `dd7d481`. No profilers were run — numeric estimates are first-principles arithmetic. Confirm with `cargo flamegraph` / Chrome DevTools / Tauri tracing before acting on the bigger-investment items.*
