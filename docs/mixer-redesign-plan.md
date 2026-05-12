# Mixer Redesign — Implementation Plan

**Status:** draft, awaiting approval
**Mockup:** https://suite.hardwavestudios.com/mixer-redesign-mockup/
**Owner:** Hardwave DAW frontend + engine teams
**Branch target:** `redesign-port-from-master`
**Tracked roadmap items:** to be carved from the existing roadmap.html backlog as we phase land.

---

## 1. Why we're rebuilding the mixer

Two pressures arrived together:

1. **Visual + UX gap.** The current mixer (`packages/daw-ui/src/components/mixer/MixerPanel.tsx`) is a flat React layout with chunky strips, no shared metaphor with FL Studio (the DAW our users come from), no dB scale on the meters, no FL-style FX rack, and no plug-in picker. It reads as a prototype.
2. **Performance ceiling.** With 500 tracks the panel lags hard on four axes: scroll/pan stutter, meter animation drag, fader/knob drag latency, and slow open-time. The current architecture mounts every strip, has each strip subscribe to the full track store, drives meters via React state at 60 Hz, and re-renders the world on every audio block.

The redesign solves both at once because the same rewrite that gives us FL Wide 2 also gives us the seams to virtualize, memoize, and offload meter rendering to canvas.

---

## 2. What ships — feature inventory

Everything below is what the user will see and use after rollout. Marked **NEW** if not in today's mixer, **CHANGE** if a refactor of existing behavior.

### 2.1 Channel strip (the basic unit)

- **NEW.** Strip is exactly **64 px** wide regardless of role (master, bus, audio, MIDI, return). Total visual density: ~22 strips visible at 1440 px viewport.
- **NEW.** Strip stacks top-to-bottom:
  1. **`s-num`** — strip number (or `M` for master, `B-A` for buses). Click area for selection.
  2. **`s-name`** — vertically-rotated track name, color-tagged by track-type tag (kick/bass/lead/fx/vox/bus/rev/master).
  3. **`s-knob`** — one **turnable pan knob** (drag to rotate ±135°, double-click to reset to center, value tooltip L100 / C / R100 appears during drag).
  4. **`s-fader-meter`** — three-column grid: **vertical fader** on the left, **dB scale** (0 / -6 / -12 / -24 / -48 with horizontal tick lines, 0 dB highlighted in accent red) in the middle, **L + R meter pair** flush together on the right.
  5. **`s-db`** — live numeric readout of the current fader value (e.g. `-3.2 dB`).
  6. **`s-sends`** — five small LED dots indicating which buses/returns this strip sends to (green = active post-fader, blue = active pre-fader / reverb routing).
- **CHANGE.** Selection state uses a 1 px inset accent border + warmer background so it reads at a glance among 500 neighbors.
- **CHANGE.** Master strip uses the **same** `.strip` shape — red-themed (`.strip.master` modifier), not a bespoke wider component. Identical fader, identical dB scale, identical meter pair, identical knob. Only the `s-num` shows `M` in solid accent.

### 2.2 Mute / Solo / Record-arm (not in mockup yet, must ship)

The mockup currently has no Mute/Solo affordance because the user replaced the static M/S buttons with the turnable knob. We still need the function. Decision (pending sign-off): **tiny pill toggles on the `s-num` row** — three 14 × 10 px pills inline next to the strip number: `M` (mute), `S` (solo), `R` (record-arm, audio tracks only). On hover the row gets a subtle bg highlight so the affordance is discoverable. Right-click on the strip opens the same toggles in a context menu plus extras (Solo Exclusive vs Additive, Reset Volume, Reset Pan, Rename, Color Tag, Delete).

### 2.3 Master strip

- Same shape as inserts (see 2.1).
- Always pinned to the far-left grid column, doesn't scroll horizontally with the inserts.
- Selectable — clicking master retargets the FX rack panel to show the master insert chain + master output routing (hardware out).
- Carries the master limiter as one of its FX rack slots — it's not a separate hardcoded UI block any more.

### 2.4 Insert strips region

- **NEW.** Horizontal flex row inside `.strips-scroll` (`overflow-x:auto`).
- **NEW.** **Virtualized.** Only the strips currently in (or near) the viewport are mounted as DOM nodes. Off-screen strips exist in state but not in the React tree. See §5 for the virtualization mechanism.
- **NEW.** Send-arrow overlay: small green / blue triangle arrows drawn between strips to indicate active sends (purely informational, FL classic).
- **CHANGE.** Drag-drop of audio clips ONTO a strip drops them on that track (existing behavior, preserved). Drag-drop of a track header reorders strips (existing).

### 2.5 FX rack panel (the right column)

- **NEW.** Fixed-width 360 px panel docked to the right of the mixer, always visible.
- **NEW.** Header reflects the **currently selected** strip (number badge + name + role line: "Insert · Stereo · routes to Master" / "Bus B · sub-group · routes to Master" / "Master bus · stereo · final output").
- **NEW.** **Send routing matrix** at the top of the panel: list of every other strip this one routes to, each row with an arrow icon, destination name, send level in dB, pre/post-fader pill toggle. Inactive routes shown dim, active in green (post) or blue (pre).
- **NEW.** **10 FX slots** stacked vertically. Each slot:
  - Slot index `01..10`
  - Status LED (green = active, red = bypassed, dashed = empty)
  - Plug-in name (full vendor + name, e.g. "Hardwave LoudLab", "FabFilter Pro-Q 3")
  - Dry/Wet knob (tiny, turnable)
  - Wet % readout
- **CHANGE.** Drag-drop to reorder slots inside the chain still works.
- **NEW.** Click an empty slot → plug-in picker flyout (see 2.6).
- **NEW.** Click a populated slot → opens that plug-in's GUI (existing behavior, preserved).
- **NEW.** Right-click populated slot → context menu: Bypass, Replace…, Remove, Copy State, Paste State, Show GUI, Solo Plugin, Make Default for new tracks.

### 2.6 Plug-in picker (flyout + modal)

Two-tier UX so common picks are one click, rare picks are still reachable:

**Tier 1 — Favorites flyout.** Click an empty slot → flyout opens beside the slot showing ~7 entries:
- 5 Hardwave native plug-ins (LoudLab, Analyser, WettBoi, KickForge, Wideboi) pinned at the top with red `H` icon.
- 2 user-favorites (most-recently-used or pinned by user).
- "Search more plug-ins… ›" link in the footer.

Click an entry → slot fills with that plug-in, flyout closes. Plug-in instance is created via the audio thread's `InsertCommand::Add` over the existing insert-chain channel; the slot reflects state once the engine confirms (or shows a transient "Loading" state while VST3 init runs).

**Tier 2 — Full picker modal.** Click "Search more" → modal opens (780 × 560 px, scrim behind):
- **Search input** at the top, debounced 80 ms. Filters by plug-in name, vendor, category, file path.
- **Category sidebar** on the left (~160 px): `All` / `Hardwave` / `Instrument` / `EQ` / `Dynamics` / `FX` / `Mastering` / `Limiter` / `Utility`. Counts shown next to each category. Custom categories defined by the plug-in scan (see §6.3).
- **Plug-in list** (right pane): each row shows icon + name + vendor + category + format badge (`NATIVE` / `VST3` / `CLAP` / `AU` / `AAX`).
- **Footer:** "Showing N plug-ins" + Cancel.
- **Keyboard:** type-to-search auto-focuses input, ↑/↓ navigates list, Enter selects, Esc closes.

### 2.7 Turnable knob primitive

- **NEW.** A `<Knob>` React primitive used everywhere a continuous parameter is controlled: pan, dry/wet, send levels, plug-in parameters, master limiter ceiling/release.
- **Drag mechanics:** pointerdown on knob captures pointer, vertical drag rotates the indicator within ±135° (default sweep). Sensitivity: 1.5 px = 1° (configurable). `Shift`-drag = fine (10× slower). `Ctrl/Cmd`-drag = coarse (3× faster).
- **Double-click** = reset to default (center for bipolar params, 0 / 100 % for unipolar).
- **Scroll wheel** over knob = increment by 1 unit per wheel notch (`Shift`-wheel = fine).
- **Value tooltip** shows the current value while dragging or hovering (`C` / `L42` / `R17` for pan, `42%` for dry/wet, `-3.5 dB` for send level, etc.).
- **CSS variable `--rot`** drives the indicator rotation so the visual stays cheap; React state holds the value, the knob applies it to the DOM via inline style on each change (not via re-render).
- **Implemented once.** Existing scattered knob/slider components in `MixerPanel`, `KickSynthEditor`, automation editor, etc. migrate to this primitive in a follow-up pass.

### 2.8 Layout shell

- Grid template: `var(--strip-w) 1fr 360px` — master column / insert scroll region / FX rack panel.
- Title bar (28 px) on top: project name, view name (`Mixer · Wide 2`), BPM / sample rate / armed indicator / CPU / strip count.
- No bottom navigator — the horizontal scrollbar is the navigator. FL Studio Wide 2 doesn't have a mini-map and neither do we.

### 2.9 Group separators

- **NEW.** A 2 px vertical accent line on the LEFT edge of strips that mark a visual group boundary. Three sources:
  - **Auto:** any bus or return strip gets the separator by default — `bus` strips use blue, `rev` strips use green, others use a dimmed accent red.
  - **Manual:** user right-clicks a strip → "Add separator before". Stored on the track as `groupBoundary: true`, persisted with the project file.
  - **Implicit:** master strip's right edge is permanent accent — already in place.
- Separator never breaks horizontal flow (it's `inset 2px 0 0 <color>` box-shadow, not a real gap), so the virtualizer math stays simple and scroll stays smooth.
- Visible in mockup; CSS class is `.strip.sep-left[.color-bus|.color-rev]` and `.strip.sep-user` for the manual variant.

### 2.10 Visual / type / color system

- **Track-type color tag** drives the rotated `s-name` color: kick (`#ff8b9d`), bass (`#ffba6a`), lead (`#a6c8ff`), fx (`#b6f0c8`), vox (`#e0a8ff`), bus (`#7adfff`), rev (`#9ad6ff italic`), master (red), default (off-white).
- **Send-dot colors:** off (`#1a1a22`), post-fader on (green `--green`), pre-fader / reverb route on (blue `--blue`).
- **dB-scale tick colors:** 0 dB tick gets `--accent` at 70 % opacity (visual ceiling), other ticks `#1f1f27`.
- **Borders:** 1 px `#2a2a32` everywhere. Max border-radius 4 px (sharp = FL).
- **Fonts:** Inter for UI, JetBrains Mono for numeric readouts (dB values, %, BPM, sample rate).
- All colors live in `:root` CSS variables so theme swaps (future) are one edit.

---

## 3. Out-of-scope (this redesign won't ship these)

These are explicit non-goals so the diff stays focused. They go on the follow-up roadmap:

- Group folder strips (FL's "Group" header strips).
- Multi-select drag (Ctrl/Shift-click to select N strips and drag-fader them together).
- Sidechain routing visualization.
- Per-strip recording arm-to-clip indicator.
- Plug-in delay compensation indicator on the strip.
- Mixer scenes / snapshots.
- Touch / multitouch fader control.
- A11y screen-reader pass (separate audit per [[accessibility_audit_pending]] memory if it exists).

---

## 4. Component breakdown — what lives where

### 4.1 Files to create

| Path | Purpose |
|---|---|
| `packages/daw-ui/src/components/mixer/MixerPanel.tsx` | **Rewrite** (existing file replaced) — top-level grid, mounts master + virtualized strip list + FX rack |
| `packages/daw-ui/src/components/mixer/ChannelStrip.tsx` | Single strip — props: `trackId`, `index`, `selected`. Subscribes to ONE track's volume/pan/mute/solo via Zustand selector. Renders `s-num` / `s-name` / `<Knob>` / `<FaderMeter>` / `s-db` / `<SendsDots>` |
| `packages/daw-ui/src/components/mixer/MasterStrip.tsx` | Thin wrapper around `ChannelStrip` with `kind="master"` — same shape, red theme, pinned positioning |
| `packages/daw-ui/src/components/mixer/FaderMeter.tsx` | The fader-column + dB-scale + meter-pair group. Owns the canvas where meters paint (see §5.3). Subscribes to per-track meter values |
| `packages/daw-ui/src/components/mixer/DbScale.tsx` | Pure visual — 5 labels + 3 tick lines, no props |
| `packages/daw-ui/src/components/mixer/SendsDots.tsx` | 5-LED row, subscribes to track's `sends` list |
| `packages/daw-ui/src/components/mixer/FxRackPanel.tsx` | Right-side 360 px panel — header + routing matrix + 10 slot list |
| `packages/daw-ui/src/components/mixer/FxSlot.tsx` | One slot row — index / led / plug-in name / wet knob |
| `packages/daw-ui/src/components/mixer/RoutingMatrix.tsx` | Send routing list above the FX slots |
| `packages/daw-ui/src/components/primitives/Knob.tsx` | Turnable knob primitive (§2.7) — used by FxSlot, ChannelStrip pan, plug-in param editors |
| `packages/daw-ui/src/components/plugin-picker/PluginPickerFlyout.tsx` | The favorites popover |
| `packages/daw-ui/src/components/plugin-picker/PluginPickerModal.tsx` | The full search modal |
| `packages/daw-ui/src/components/plugin-picker/PluginIcon.tsx` | Small 18 px icon (native vs 3rd-party) used in flyout + modal |
| `packages/daw-ui/src/stores/mixerSelectionStore.ts` | Single source of truth for "which strip is currently selected" — drives the FX rack |
| `packages/daw-ui/src/stores/pluginCatalogStore.ts` | List of every scanned plug-in (id, name, vendor, category, format, path) — fetched from Tauri once at boot, refreshed on user request |
| `packages/daw-ui/src/stores/pluginFavoritesStore.ts` | User pins + MRU list driving the flyout's 7 entries |
| `src-tauri/src/commands/plugin_catalog.rs` | Tauri commands `list_plugins`, `rescan_plugins`, returns serialized catalog |

### 4.2 Files to delete or strip down

| Path | Action |
|---|---|
| `packages/daw-ui/src/components/mixer/MixerPanel.tsx` (current) | Replace contents |
| Existing `MasterStrip*`, `SlotPanel*` components if present | Delete after the new equivalents land and pass functional smoke |
| Any duplicated knob/fader components scattered across `KickSynthEditor` etc. | Will migrate to `<Knob>` in a follow-up sweep, not this PR |

### 4.3 Files to touch (small edits)

| Path | What changes |
|---|---|
| `packages/daw-ui/src/stores/trackStore.ts` | Add selectors `useTrackById(id)`, `useTrackVolume(id)`, `useTrackPan(id)`, `useTrackMute(id)`, `useTrackSolo(id)`, `useTrackSends(id)` — fine-grained so a strip only re-renders when its own values change |
| `packages/daw-ui/src/stores/meterStore.ts` | Switch from React-driving updates to a ref-only / observer model (see §5.3) |
| `src-tauri/src/commands/plugins.rs` | Wire new "create plug-in by descriptor id" path that the picker uses |
| `src-tauri/src/lib.rs` | Register the new plugin_catalog commands |
| `crates/hardwave-plugin-host/src/lib.rs` | Expose the plug-in scanning + descriptor enumeration (already partially exists for VST3/CLAP — needs surfacing) |

---

## 5. Performance — how we hit 60 fps with 500 strips

### 5.1 Targets

| Scenario | Today (est.) | Target |
|---|---|---|
| Mixer open with 500 tracks | 1.5–3 s blocking | < 300 ms first paint, full virtualized |
| Horizontal scroll FPS | 15–25 fps | 60 fps (16 ms frame) |
| Meter animation cost with all strips visible | 35–50 % CPU on the renderer | < 5 % CPU |
| Fader drag latency | 40–80 ms input → repaint | < 16 ms |
| Memory with 500 strips loaded | likely 400–600 MB | < 250 MB |

### 5.2 Virtualization

- Use **`@tanstack/react-virtual`** (already a peer dep of several libs, lightweight, ~3 kb, supports horizontal lists with `overscan`).
- The `MixerPanel` renders the master + FX rack directly. The middle column renders a virtualizer that mounts only `visibleStrips + overscan(8)` strips.
- Strip width is fixed (64 px) so the virtualizer math is trivial — no measurement pass needed.
- Off-screen strips don't subscribe to stores. When they scroll into the overscan window, they mount, subscribe, and have ~120 ms before they're actually visible to settle.

### 5.3 Meter rendering — direct canvas, never through React

Today the meter values flow `audio thread → IPC event → React state → component re-render → DOM style update` for every meter on every frame. With 500 strips at 60 fps that's 30,000 React updates per second. It will never be fast.

The new model:

- Each `FaderMeter` component creates a `<canvas>` once on mount.
- A single global **`meterStream`** singleton holds the latest L/R peak + RMS values per track in a `Map<trackId, Float32Array(4)>` that the audio thread writes to via a single Tauri event channel.
- `meterStream` runs **one** `requestAnimationFrame` loop at 60 fps that iterates only the **currently visible** track ids (from the virtualizer) and paints each canvas directly.
- No React re-renders. The component sets up the canvas, registers its `trackId`, and the rAF loop owns repaint.
- The dB scale is purely static CSS (no canvas needed).

### 5.4 Fader / knob drag

- `<Knob>` and `<Fader>` use `pointerdown` → `setPointerCapture` → `pointermove` → `pointerup`.
- During drag, the visual indicator (`--rot` for knob, `top%` for fader cap) updates via **direct DOM mutation** (`element.style.setProperty`) inside the move handler — no React state on the hot path.
- A **trailing-rAF-batched** dispatch fires the actual `trackStore.setVolume(id, value)` / `engine.set_volume(id, value)` call once per frame at most.
- On `pointerup` the final value commits to React state so it becomes the source of truth for non-dragging consumers.

### 5.5 Selector hygiene

- `trackStore` exposes the new fine-grained selectors listed in §4.3.
- Every `ChannelStrip` subscribes only to the fields it renders.
- **No `useTrackStore()` without a selector.** The old pattern of `const tracks = useTrackStore(s => s.tracks)` returns a fresh array reference on every update and re-renders every consumer — banned by ESLint rule `no-bare-zustand-track-store` (we add this rule).
- Strip mute / solo state changes from the selection store won't cascade — selection is its own store (`mixerSelectionStore`).

### 5.6 Memoization

- Every leaf component (`ChannelStrip`, `FxSlot`, `Knob`, `SendsDots`) is wrapped in `React.memo` with a custom `propsAreEqual` that compares only primitive props (id, index, value).
- Parent passes stable callbacks (`useCallback`) — handler factories live one level up.

### 5.7 Mount cost

- The FX rack mounts but its 10 slots only request plug-in metadata when a strip is selected and the chain is non-empty.
- The picker modal **never mounts** until the user clicks "Search more" the first time. Lazy-imported chunk.
- `pluginCatalogStore` does **not** scan plug-ins on app boot — it hydrates from a cached `plugins.json` written on the last successful scan. Background refresh runs after the first idle frame.

---

## 6. Tauri / Rust changes

### 6.1 Plug-in catalog command

```rust
#[tauri::command]
pub fn list_plugins(state: State<AppState>) -> Vec<PluginDescriptor> { /* … */ }

#[tauri::command]
pub fn rescan_plugins(state: State<AppState>) -> Result<usize, String> { /* … */ }
```

`PluginDescriptor` is the same struct the engine already uses internally for `InsertCommand::Add`, serialized for the frontend with:
- `id` (stable hash of vendor + name + format)
- `name`
- `vendor`
- `category` (one of: Instrument, EQ, Dynamics, FX, Mastering, Limiter, Utility — derived from VST3 subcategory string or the native plug-in's declared category)
- `format` (`Native` / `Vst3` / `Clap` / `Au` / `Aax`)
- `path` (filesystem path, for diagnostics + scan-cache invalidation)
- `is_instrument` (boolean)

### 6.2 Plug-in instantiation by descriptor

The picker sends `descriptor_id` to a new Tauri command `add_plugin_to_chain(track_id, slot_index, descriptor_id)`. The handler:
1. Looks up the descriptor.
2. Asks `hardwave-plugin-host` to instantiate.
3. Sends `InsertCommand::Add { track_id, slot_index, plugin }` over the existing insert-chain channel.
4. Returns the new slot's `slot_id` for the UI to reference.

This is a thin layer over what `plugins.rs::add_plugin` already does — we add the descriptor lookup but reuse the channel + audio-thread integration.

### 6.3 Plug-in scanning

`hardwave-plugin-host` already has VST3 + CLAP scan functions. We expose them via `rescan_plugins`. The scan:
1. Walks `~/Library/Audio/Plug-Ins/VST3/`, `/Library/Audio/Plug-Ins/VST3/`, the user's configured scan paths, and the corresponding CLAP / AU paths on macOS / Windows / Linux.
2. For each candidate, instantiates briefly to read metadata, then destroys.
3. Writes results to `<appdata>/hardwave/plugins.json` as the cache.
4. Emits `plugin-catalog-updated` event so the frontend store refreshes.

Native Hardwave plug-ins are baked into the catalog statically — no scan needed.

---

## 7. State model summary

```ts
// mixerSelectionStore
{ selectedTrackId: string | null;  selectTrack(id): void }

// trackStore (existing, with new fine-grained selectors)
{
  tracks: Track[];
  setVolume(id, dB): void;
  setPan(id, value: -100..100): void;
  setMute(id, on: boolean): void;
  setSolo(id, on: boolean): void;
  setRecordArm(id, on: boolean): void;
}

// pluginCatalogStore
{
  plugins: PluginDescriptor[];
  loaded: boolean;
  rescan(): Promise<void>;
  byCategory(cat: string): PluginDescriptor[];
  byId(id: string): PluginDescriptor | undefined;
  search(q: string): PluginDescriptor[];
}

// pluginFavoritesStore
{
  pinned: string[];  // descriptor ids
  recent: string[];  // mru
  pin(id): void;
  unpin(id): void;
  markUsed(id): void;
}
```

Per-track meter state stays out of React entirely (see §5.3).

---

## 8. Phasing — what ships when

Each phase is independently shippable as a tagged release (`patch`, per `feedback_daw_versioning.md`). User can opt into the new mixer behind a feature flag (`settings.experimentalMixer = true`) until phase 3 lands.

### Phase 1 — Visual structure & turnable knob primitive
- Land `Knob.tsx` primitive (§2.7).
- Land `FaderMeter.tsx`, `DbScale.tsx`, `SendsDots.tsx`.
- Land new `ChannelStrip.tsx`.
- New `MixerPanel.tsx` rendering master + non-virtualized strip list + empty FX rack panel.
- Existing functionality preserved — fader, pan, mute, solo wire to existing store actions.
- **Not** virtualized yet, **not** canvas-meter yet. Just the shape.
- Ships as `v0.160.0` (minor — new UI feature) behind `experimentalMixer` flag.

### Phase 2 — Master strip parity + selection
- `MasterStrip.tsx` using the same `.strip` shape.
- `mixerSelectionStore` drives which strip the FX rack shows.
- Click any strip (master or insert) → FX rack header retargets.
- Master limiter becomes "just an FX slot" — remove the bespoke limiter UI.
- Ships as `v0.160.1`.

### Phase 3 — FX rack panel + plug-in picker
- `FxRackPanel.tsx`, `FxSlot.tsx`, `RoutingMatrix.tsx`.
- `PluginPickerFlyout.tsx` + `PluginPickerModal.tsx`.
- `pluginCatalogStore`, `pluginFavoritesStore`.
- Tauri commands `list_plugins`, `rescan_plugins`, `add_plugin_to_chain` wired.
- Click empty slot → flyout → pick → engine `InsertCommand::Add`.
- Click "Search more" → modal with categorized list, search, format badges.
- Flag flipped to default-on at the end of this phase; old MixerPanel removed.
- Ships as `v0.161.0` (minor — picker is a new feature).

### Phase 4 — Performance pass
- `@tanstack/react-virtual` for horizontal strip virtualization.
- Canvas meter rendering via `meterStream` singleton.
- Fine-grained `trackStore` selectors + ESLint rule.
- React.memo on all leaf components.
- RAF-batched fader/knob drag commits.
- Lazy-mount FX rack and picker modal.
- Measure: 500-strip session must hit 60 fps scroll, < 5 % meter CPU, < 16 ms drag latency.
- Ships as `v0.161.1` (patch — perf fixes).

### Phase 5 — Polish & follow-ups
- Mute/Solo/Record-arm pills on `s-num` row (§2.2).
- Right-click context menus on strip + FX slot.
- Send-arrow overlay between strips.
- Color-tag editor (user can set their own track-type tag).
- Migrate remaining knob/slider components across the app to the new `<Knob>` primitive.
- Ships as `v0.161.2`.

---

## 9. Risks & open questions

### Risks

1. **Plug-in scan time on first launch.** Some users have 1000+ VST3s. Scan can take 30–90 s. Mitigation: scan runs in background, cached `plugins.json` hydrates instantly, user sees "scanning…" pill in the title bar.
2. **Virtualizer + horizontal scroll on touchpad inertia (macOS).** Some virtualizers stutter on momentum scroll. Test on M-series MacBooks before sign-off. If `@tanstack/react-virtual` struggles, fall back to `react-window-horizontal`.
3. **Canvas meter sync with the audio thread.** If the Tauri event channel can't sustain 60 Hz with 500 entries, we may need to batch meter snapshots into a single `Float32Array` blob per frame and `postMessage` it. Already a fallback in the design.
4. **Migration of existing projects.** Loading a v0.159 project into the new mixer must preserve all volumes, pans, send routings, plug-in chains. Covered by existing project serialization — no schema change needed for phases 1–4.

### Open questions (need user decision before phase 1 starts)

1. **Mute/Solo/Record-arm placement.** Inline pills on `s-num`, separate row below sends, right-click only, or some mix? Plan currently assumes pills on `s-num`.
2. **Default sweep angle of pan knob — ±135° (FL) or ±150° (Ableton)?** Plan assumes ±135°.
3. **Strip width — 64 px (mockup) or 72 px (FL default)?** Plan assumes 64 px. 72 px would reduce strips-visible-at-once from 22 to 19.
4. **Color-tag system — user-editable or fixed by track type?** Plan assumes user-editable in Phase 5, fixed by inferred type in phases 1–4.
5. **Plug-in scan paths — pull from existing settings panel or hard-code?** Plan assumes existing settings panel (no new UI in this PR).
6. **Should the picker remember the last category per slot, or always default to All?** Plan assumes always-default-to-All.

---

## 10. Sign-off checklist before phase 1 starts

- [ ] Mockup approved by user (in progress — last iteration: L+R meters + dB scale + turnable knob)
- [ ] Mute/Solo/Record-arm placement decided (Q1 above)
- [ ] Strip width confirmed (Q3 above)
- [ ] `@tanstack/react-virtual` added to `package.json`
- [ ] Existing roadmap items that overlap (mixer-related rows) carved out and reassigned to this plan's phases so we don't double-track
- [ ] Functional smoke tests updated to exercise the new selection flow + picker

---

## 11. Estimated effort

Rough engineering days, single dev:

| Phase | Days |
|---|---|
| 1 — Visual + knob primitive | 3–4 |
| 2 — Master parity + selection | 1 |
| 3 — FX rack + picker + Tauri commands | 4–5 |
| 4 — Performance pass | 3–4 |
| 5 — Polish | 2–3 |
| **Total** | **13–17 days** |

Risk margin: +30 % for plug-in scan edge cases on Windows.

---

---

## 12. Related fix — Playlist click model (not strictly mixer, ships separately)

User-reported bug fixed alongside this redesign because it shares the "FL Studio interaction parity" goal:

- **Before:** every left-click on empty playlist space flashed a teal dashed edit-cursor line at `#14B8A6` (`Arrangement.tsx:467-488`, triggered by `setEditCursor()` at `:794`) and immediately armed a rubber-band drag. A zero-distance click left the teal line behind.
- **After (v0.159.2 patch):** mousedown on empty area enters a new `'pending-empty'` drag mode. The cursor doesn't move, the rubber band doesn't render, and nothing visual happens until `pointermove` crosses **4 px** from the start point. Crossing the threshold promotes to `'rubber'` and commits the edit cursor at the original mousedown tick. Releasing without crossing the threshold = pure no-op.
- Audio-clip paste-paint via the picker (`pickerStore.kind === 'audioClip'`) still fires immediately on mousedown — unchanged.

### Tool-mode foundation (deferred to v0.160.0)

To unlock keyboard tool-switching (CTRL+B duplicate, B paint, S slice, D delete, E select, etc. like FL):

- New `packages/daw-ui/src/stores/playlistToolStore.ts` (Zustand):
  ```ts
  type PlaylistTool = 'draw' | 'paint' | 'slice' | 'delete' | 'mute' | 'slip' | 'select' | 'zoom'
  { tool: PlaylistTool; setTool(t): void }
  ```
- `Toolbar.tsx:44` swaps its local `useState<Tool>` for `usePlaylistToolStore` — currently the tool state is purely decorative there.
- `Arrangement.tsx` mousedown branches on `tool`:
  - `draw` (default): the pending-empty / paste model from v0.159.2.
  - `select`: rubber-band on every press (no paint, no threshold).
  - `slice`: split clip at tick.
  - `delete`: remove clip under cursor.
  - others: TBD per FL parity table.
- `App.tsx` registers keybinds via the existing `useShortcutsStore.matchEvent` plumbing (already wired at `:915-917`). Default bindings: `B`=paint, `S`=slice, `D`=delete, `E`=select, `T`=mute, `Ctrl+B`=duplicate selection, `Ctrl+L`=loop selection, `Y`=slip.

### Risks of the fix

- `editCursorTicks` is the Ctrl+V paste origin (`App.tsx:817`, `:942`, `PianoRoll.tsx:1236,1335,1386`). With v0.159.2 it only moves on a deliberate drag-promoted click, otherwise stays where it was. Users who relied on click-to-position-paste need to either drag a tiny bit or use the ruler (which still sets transport position) — added to the changelog so it's not a silent UX regression.
- Picker selections of kind `'pattern'` or `'automation'` still fall through to the new pending-empty mode (no paste path exists for them yet). Scoped into the v0.160.0 tool-mode work.

*This plan supersedes any earlier informal mixer design notes. Updates land here, not in chat threads.*
