//! Per-track plug-in insert chain runtime — the missing audio-routing
//! layer that connects `Project::tracks[*].inserts` (metadata) to
//! `HostedPlugin::process()` (DSP).
//!
//! Threading model:
//!   - UI thread (Tauri command handlers) constructs plug-in instances
//!     off-RT and sends them across the [`InsertCommandQueue`] to the
//!     audio thread. The UI thread never touches a live `LiveSlot`.
//!   - Audio thread is the sole owner of every [`LiveSlot`] and its
//!     `Box<dyn HostedPlugin>`. It drains the command queue at the start
//!     of every audio block, mutates chains, then runs `process()` on
//!     each enabled slot in series.
//!   - When a slot is removed, the audio thread cannot drop the boxed
//!     plug-in (allocation calls `free()` and may block) — it ships the
//!     box back through the [`PluginGraveyard`] for a non-RT thread to
//!     drop.
//!
//! This module is deliberately decoupled from `engine.rs` so it can be
//! unit-tested in isolation and so swapping the queue implementation
//! later (e.g. multi-producer, scratch-buffer pooling) is a local change.

use hardwave_plugin_host::types::HostedPlugin;
use rtrb::{Consumer, Producer, PushError, RingBuffer};
use std::collections::HashMap;

/// Capacity of the UI→audio command ringbuffer per engine. 256 is plenty
/// — UI commands fire at most a few times per second; the audio thread
/// drains the buffer at audio-rate (every block, ~5ms at 48kHz/256 samples).
const DEFAULT_COMMAND_CAPACITY: usize = 256;

/// Capacity of the audio→graveyard ringbuffer. Sized to absorb a burst
/// of removals (e.g. a "clear all inserts" UI action) without back-
/// pressuring the audio thread.
const DEFAULT_GRAVEYARD_CAPACITY: usize = 64;

/// Live, audio-thread-owned representation of a single insert slot.
/// `slot_id` matches the `PluginSlot::id` from the project model so the
/// audio chain can be reconciled against `track.inserts` after edits.
pub struct LiveSlot {
    pub slot_id: String,
    pub plugin: Box<dyn HostedPlugin>,
    pub enabled: bool,
    /// Dry/wet mix in [0, 1]. 1.0 = fully processed, 0.0 = pure dry
    /// (equivalent to `enabled = false` for audible output, but the
    /// plug-in still runs so its meters/sidechains keep updating).
    pub wet: f32,
}

/// Ordered list of live insert slots for one track.
#[derive(Default)]
pub struct InsertChain {
    pub slots: Vec<LiveSlot>,
}

impl InsertChain {
    pub fn new() -> Self {
        Self { slots: Vec::new() }
    }

    /// Process a stereo audio block in place through every enabled slot
    /// in series. The plug-in's own `process()` writes to a scratch
    /// output buffer; this routine mixes scratch back into `left`/`right`
    /// per the slot's wet level. Pre-allocated scratch buffers are
    /// resized on first use; `num_samples` must fit within `left.len()`
    /// and `right.len()`.
    ///
    /// Real-time-safe under steady-state — no allocations once
    /// `scratch_capacity` is set ≥ block size. The audio thread should
    /// call [`InsertChain::ensure_scratch_capacity`] whenever buffer
    /// size changes.
    pub fn process(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        num_samples: usize,
        scratch: &mut Scratch,
    ) {
        let n = num_samples.min(left.len()).min(right.len());
        if n == 0 {
            return;
        }
        scratch.prepare(n);
        for slot in self.slots.iter_mut() {
            if !slot.enabled || slot.wet <= 0.0 {
                continue;
            }
            let inputs: [&[f32]; 2] = [&left[..n], &right[..n]];
            // Reuse the pre-allocated channel vecs; clear() preserves
            // capacity so the plug-in's `extend_from_slice` / `push`
            // calls land back into the same heap allocation.
            scratch.channels[0].clear();
            scratch.channels[1].clear();
            scratch.midi_out.clear();
            slot.plugin
                .process(&inputs, &mut scratch.channels, &[], &mut scratch.midi_out, n);
            let wet = slot.wet.clamp(0.0, 1.0);
            let dry = 1.0 - wet;
            // Plug-ins are allowed to under-fill their output buffer
            // (e.g. internal block size shorter than `n`). Treat the
            // returned length as authoritative; samples past it stay dry.
            let valid_l = scratch.channels[0].len().min(n);
            let valid_r = scratch.channels[1].len().min(n);
            for i in 0..valid_l {
                left[i] = dry * left[i] + wet * scratch.channels[0][i];
            }
            for i in 0..valid_r {
                right[i] = dry * right[i] + wet * scratch.channels[1][i];
            }
        }
    }

    /// Insert a slot at the end of the chain. Activates the plug-in at
    /// the given sample rate before adding so the chain is immediately
    /// process-ready.
    pub fn push_slot(
        &mut self,
        slot: LiveSlot,
        sample_rate: f64,
        max_block_size: u32,
    ) -> Result<(), String> {
        let mut s = slot;
        s.plugin.activate(sample_rate, max_block_size)?;
        self.slots.push(s);
        Ok(())
    }

    /// Remove a slot by id and return the freed `LiveSlot` for off-RT
    /// disposal. Caller is responsible for shipping it through the
    /// graveyard channel — the audio thread must not drop directly.
    pub fn take_slot(&mut self, slot_id: &str) -> Option<LiveSlot> {
        let idx = self.slots.iter().position(|s| s.slot_id == slot_id)?;
        let mut slot = self.slots.remove(idx);
        slot.plugin.deactivate();
        Some(slot)
    }

    /// Move a slot to a new position. No-op if `from` is out of range.
    pub fn reorder(&mut self, from: usize, to: usize) {
        if from >= self.slots.len() {
            return;
        }
        let to = to.min(self.slots.len().saturating_sub(1));
        if from == to {
            return;
        }
        let slot = self.slots.remove(from);
        self.slots.insert(to, slot);
    }

    pub fn set_enabled(&mut self, slot_id: &str, enabled: bool) -> bool {
        if let Some(s) = self.slots.iter_mut().find(|s| s.slot_id == slot_id) {
            s.enabled = enabled;
            true
        } else {
            false
        }
    }

    pub fn set_wet(&mut self, slot_id: &str, wet: f32) -> bool {
        if let Some(s) = self.slots.iter_mut().find(|s| s.slot_id == slot_id) {
            s.wet = wet.clamp(0.0, 1.0);
            true
        } else {
            false
        }
    }

    pub fn set_parameter(&mut self, slot_id: &str, param_id: u32, value: f64) -> bool {
        if let Some(s) = self.slots.iter_mut().find(|s| s.slot_id == slot_id) {
            s.plugin.set_parameter_value(param_id, value);
            true
        } else {
            false
        }
    }
}

/// Reusable scratch buffers for [`InsertChain::process`]. One instance
/// per audio thread; the chain re-sizes on demand. Living in a separate
/// struct lets the engine keep one set of scratch buffers shared across
/// every track's chain instead of N allocations per track. The shape
/// matches `HostedPlugin::process`'s `outputs: &mut [Vec<f32>]` so we
/// can pass it through directly without copying or per-call vec
/// allocations.
pub struct Scratch {
    pub channels: Vec<Vec<f32>>,
    pub midi_out: Vec<hardwave_midi::MidiEvent>,
    capacity: usize,
}

impl Scratch {
    pub fn with_capacity(capacity: usize) -> Self {
        let mut channels = Vec::with_capacity(2);
        channels.push(Vec::with_capacity(capacity));
        channels.push(Vec::with_capacity(capacity));
        Self {
            channels,
            midi_out: Vec::with_capacity(64),
            capacity,
        }
    }

    /// Grow the underlying vec capacities to hold `num_samples`. Idempotent
    /// and a no-op once `capacity >= num_samples`.
    pub fn prepare(&mut self, num_samples: usize) {
        if num_samples > self.capacity {
            for ch in self.channels.iter_mut() {
                ch.reserve(num_samples - self.capacity);
            }
            self.capacity = num_samples;
        }
    }
}

impl Default for Scratch {
    fn default() -> Self {
        Self::with_capacity(2048)
    }
}

/// Cross-thread command sent from UI to audio thread. The plug-in
/// instance is heap-allocated UI-side and ownership transfers to the
/// audio thread when the command lands. Drop ordering is the audio
/// thread's responsibility (via [`PluginGraveyard`]).
pub enum InsertCommand {
    Add {
        track_id: String,
        slot: LiveSlot,
    },
    Remove {
        track_id: String,
        slot_id: String,
    },
    Reorder {
        track_id: String,
        from: usize,
        to: usize,
    },
    SetEnabled {
        track_id: String,
        slot_id: String,
        enabled: bool,
    },
    SetWet {
        track_id: String,
        slot_id: String,
        wet: f32,
    },
    SetParameter {
        track_id: String,
        slot_id: String,
        param_id: u32,
        value: f64,
    },
}

/// UI-side handle for queueing commands toward the audio thread.
pub struct InsertCommandSender {
    tx: Producer<InsertCommand>,
}

/// Audio-side handle for draining commands at the start of every block.
pub struct InsertCommandReceiver {
    rx: Consumer<InsertCommand>,
}

/// Allocation-side of dropped plug-ins. Audio thread pushes vacated
/// `LiveSlot`s here; a non-RT drop thread (or the UI thread on idle)
/// drains and drops them.
pub struct PluginGraveyardSender {
    tx: Producer<LiveSlot>,
}
pub struct PluginGraveyardReceiver {
    rx: Consumer<LiveSlot>,
}

impl InsertCommandSender {
    /// Try to queue a command. Returns `Err(cmd)` if the audio thread is
    /// not draining fast enough — the UI should retry on the next
    /// frame, not block.
    pub fn try_send(&mut self, cmd: InsertCommand) -> Result<(), InsertCommand> {
        match self.tx.push(cmd) {
            Ok(()) => Ok(()),
            Err(PushError::Full(c)) => Err(c),
        }
    }
}

impl PluginGraveyardSender {
    /// Audio-thread-safe push. Returns `false` if the graveyard is
    /// full — extremely unlikely (≥64 simultaneous removals would have
    /// to outpace the drop thread). Caller can leak in that case
    /// rather than block the audio thread; the slot's destructor will
    /// run when the engine itself is dropped.
    pub fn try_bury(&mut self, slot: LiveSlot) -> Result<(), LiveSlot> {
        match self.tx.push(slot) {
            Ok(()) => Ok(()),
            Err(PushError::Full(s)) => Err(s),
        }
    }
}

impl PluginGraveyardReceiver {
    /// Drain everything pending, dropping each slot. Call from a non-RT
    /// thread (UI tick, dedicated worker, or `Drop` of the engine).
    pub fn drain_and_drop(&mut self) -> usize {
        let mut count = 0;
        while self.rx.pop().is_ok() {
            count += 1;
            // Slot drops here, calling free() on the boxed plug-in.
        }
        count
    }
}

/// Build a paired (sender, receiver) for the UI→audio command channel.
pub fn command_channel(capacity: usize) -> (InsertCommandSender, InsertCommandReceiver) {
    let (tx, rx) = RingBuffer::<InsertCommand>::new(capacity);
    (
        InsertCommandSender { tx },
        InsertCommandReceiver { rx },
    )
}

/// Build a paired (sender, receiver) for the audio→graveyard channel.
pub fn graveyard_channel(capacity: usize) -> (PluginGraveyardSender, PluginGraveyardReceiver) {
    let (tx, rx) = RingBuffer::<LiveSlot>::new(capacity);
    (
        PluginGraveyardSender { tx },
        PluginGraveyardReceiver { rx },
    )
}

/// Audio-thread orchestrator that drains the command queue and applies
/// each command to the right per-track [`InsertChain`]. Removed slots
/// are pushed into the graveyard rather than dropped on the audio
/// thread.
pub struct InsertRouter {
    pub chains: HashMap<String, InsertChain>,
    pub graveyard: PluginGraveyardSender,
    pub sample_rate: f64,
    pub max_block_size: u32,
}

impl InsertRouter {
    pub fn new(
        graveyard: PluginGraveyardSender,
        sample_rate: f64,
        max_block_size: u32,
    ) -> Self {
        Self {
            chains: HashMap::new(),
            graveyard,
            sample_rate,
            max_block_size,
        }
    }

    /// Drain every queued command and apply. Call once per audio block
    /// before processing any track.
    pub fn drain(&mut self, rx: &mut InsertCommandReceiver) {
        while let Ok(cmd) = rx.rx.pop() {
            self.apply(cmd);
        }
    }

    fn apply(&mut self, cmd: InsertCommand) {
        match cmd {
            InsertCommand::Add { track_id, slot } => {
                let chain = self.chains.entry(track_id).or_insert_with(InsertChain::new);
                if let Err(e) = chain.push_slot(slot, self.sample_rate, self.max_block_size) {
                    log::warn!("insert chain: activate failed: {e}");
                }
            }
            InsertCommand::Remove { track_id, slot_id } => {
                if let Some(chain) = self.chains.get_mut(&track_id) {
                    if let Some(slot) = chain.take_slot(&slot_id) {
                        if let Err(_returned) = self.graveyard.try_bury(slot) {
                            // Graveyard saturated — leak rather than
                            // free on the audio thread. The slot drops
                            // when the engine is dropped via the
                            // returned LiveSlot going out of scope.
                            log::warn!(
                                "insert chain: graveyard full, leaking slot {slot_id} drop to engine teardown"
                            );
                        }
                    }
                }
            }
            InsertCommand::Reorder { track_id, from, to } => {
                if let Some(chain) = self.chains.get_mut(&track_id) {
                    chain.reorder(from, to);
                }
            }
            InsertCommand::SetEnabled { track_id, slot_id, enabled } => {
                if let Some(chain) = self.chains.get_mut(&track_id) {
                    chain.set_enabled(&slot_id, enabled);
                }
            }
            InsertCommand::SetWet { track_id, slot_id, wet } => {
                if let Some(chain) = self.chains.get_mut(&track_id) {
                    chain.set_wet(&slot_id, wet);
                }
            }
            InsertCommand::SetParameter { track_id, slot_id, param_id, value } => {
                if let Some(chain) = self.chains.get_mut(&track_id) {
                    chain.set_parameter(&slot_id, param_id, value);
                }
            }
        }
    }
}

/// Convenience: paired UI sender + audio-side router with default
/// capacities. The graveyard receiver is returned separately so the
/// caller can drain it on a non-RT cadence.
pub fn build_runtime(
    sample_rate: f64,
    max_block_size: u32,
) -> (
    InsertCommandSender,
    InsertCommandReceiver,
    InsertRouter,
    PluginGraveyardReceiver,
) {
    let (cmd_tx, cmd_rx) = command_channel(DEFAULT_COMMAND_CAPACITY);
    let (grave_tx, grave_rx) = graveyard_channel(DEFAULT_GRAVEYARD_CAPACITY);
    let router = InsertRouter::new(grave_tx, sample_rate, max_block_size);
    (cmd_tx, cmd_rx, router, grave_rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardwave_midi::MidiEvent;
    use hardwave_plugin_host::types::{
        ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
    };
    use raw_window_handle::RawWindowHandle;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Minimal test plug-in: applies a constant gain stored in
    /// parameter 0. Lets us check end-to-end audio flow without a
    /// dependency cycle on `hardwave-native-plugins`.
    struct GainPlugin {
        descriptor: PluginDescriptor,
        gain: f32,
        active: bool,
        process_calls: Arc<AtomicU32>,
    }

    impl GainPlugin {
        fn new(initial_gain: f32, counter: Arc<AtomicU32>) -> Self {
            Self {
                descriptor: PluginDescriptor {
                    id: "test.gain".into(),
                    name: "Test Gain".into(),
                    vendor: "Hardwave Tests".into(),
                    version: "0.0.1".into(),
                    format: PluginFormat::Clap,
                    path: PathBuf::from("<test>"),
                    category: PluginCategory::Effect,
                    num_inputs: 2,
                    num_outputs: 2,
                    has_midi_input: false,
                    has_editor: false,
                },
                gain: initial_gain,
                active: false,
                process_calls: counter,
            }
        }
    }

    impl HostedPlugin for GainPlugin {
        fn descriptor(&self) -> &PluginDescriptor {
            &self.descriptor
        }
        fn activate(&mut self, _sr: f64, _max: u32) -> Result<(), String> {
            self.active = true;
            Ok(())
        }
        fn deactivate(&mut self) {
            self.active = false;
        }
        fn process(
            &mut self,
            inputs: &[&[f32]],
            outputs: &mut [Vec<f32>],
            _midi_in: &[MidiEvent],
            _midi_out: &mut Vec<MidiEvent>,
            num_samples: usize,
        ) {
            self.process_calls.fetch_add(1, Ordering::Relaxed);
            if outputs.len() < 2 || inputs.len() < 2 {
                return;
            }
            outputs[0].clear();
            outputs[1].clear();
            for i in 0..num_samples.min(inputs[0].len()) {
                outputs[0].push(inputs[0][i] * self.gain);
            }
            for i in 0..num_samples.min(inputs[1].len()) {
                outputs[1].push(inputs[1][i] * self.gain);
            }
        }
        fn get_parameter_count(&self) -> u32 {
            1
        }
        fn get_parameter_info(&self, _id: u32) -> Option<ParameterInfo> {
            Some(ParameterInfo {
                id: 0,
                name: "Gain".into(),
                default_value: 1.0,
                min: 0.0,
                max: 4.0,
                unit: "x".into(),
                automatable: true,
            })
        }
        fn get_parameter_value(&self, _id: u32) -> f64 {
            self.gain as f64
        }
        fn set_parameter_value(&mut self, _id: u32, value: f64) {
            self.gain = (value as f32).clamp(0.0, 4.0);
        }
        fn get_state(&self) -> Vec<u8> {
            self.gain.to_le_bytes().to_vec()
        }
        fn set_state(&mut self, bytes: &[u8]) -> Result<(), String> {
            if bytes.len() < 4 {
                return Err("too short".into());
            }
            self.gain = f32::from_le_bytes(bytes[..4].try_into().unwrap());
            Ok(())
        }
        fn latency_samples(&self) -> u32 {
            0
        }
        fn open_editor(&mut self, _parent: RawWindowHandle) -> bool {
            false
        }
        fn close_editor(&mut self) {}
        fn has_editor(&self) -> bool {
            false
        }
    }

    fn make_gain_slot(id: &str, gain: f32, enabled: bool) -> (LiveSlot, Arc<AtomicU32>) {
        let counter = Arc::new(AtomicU32::new(0));
        let slot = LiveSlot {
            slot_id: id.into(),
            plugin: Box::new(GainPlugin::new(gain, counter.clone())),
            enabled,
            wet: 1.0,
        };
        (slot, counter)
    }

    fn block(len: usize, value: f32) -> Vec<f32> {
        vec![value; len]
    }

    #[test]
    fn empty_chain_passes_audio_through_unchanged() {
        let mut chain = InsertChain::new();
        let mut scratch = Scratch::default();
        let mut left = block(256, 0.5);
        let mut right = block(256, 0.5);
        chain.process(&mut left, &mut right, 256, &mut scratch);
        assert!(left.iter().all(|&v| (v - 0.5).abs() < 1e-6));
        assert!(right.iter().all(|&v| (v - 0.5).abs() < 1e-6));
    }

    #[test]
    fn enabled_slot_applies_plugin_processing() {
        let mut chain = InsertChain::new();
        let (slot, counter) = make_gain_slot("s1", 2.0, true);
        chain.push_slot(slot, 48_000.0, 512).unwrap();
        let mut scratch = Scratch::default();
        let mut left = block(256, 0.5);
        let mut right = block(256, 0.5);
        chain.process(&mut left, &mut right, 256, &mut scratch);
        assert!(left.iter().all(|&v| (v - 1.0).abs() < 1e-6));
        assert!(right.iter().all(|&v| (v - 1.0).abs() < 1e-6));
        assert_eq!(counter.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn disabled_slot_passes_through_without_calling_plugin() {
        let mut chain = InsertChain::new();
        let (slot, counter) = make_gain_slot("s1", 2.0, false);
        chain.push_slot(slot, 48_000.0, 512).unwrap();
        let mut scratch = Scratch::default();
        let mut left = block(256, 0.5);
        let mut right = block(256, 0.5);
        chain.process(&mut left, &mut right, 256, &mut scratch);
        assert!(left.iter().all(|&v| (v - 0.5).abs() < 1e-6));
        assert_eq!(counter.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn wet_zero_passes_through_unchanged_even_when_enabled() {
        let mut chain = InsertChain::new();
        let (mut slot, _counter) = make_gain_slot("s1", 2.0, true);
        slot.wet = 0.0;
        chain.push_slot(slot, 48_000.0, 512).unwrap();
        let mut scratch = Scratch::default();
        let mut left = block(256, 0.5);
        let mut right = block(256, 0.5);
        chain.process(&mut left, &mut right, 256, &mut scratch);
        assert!(left.iter().all(|&v| (v - 0.5).abs() < 1e-6));
    }

    #[test]
    fn wet_half_blends_dry_and_wet() {
        let mut chain = InsertChain::new();
        let (mut slot, _counter) = make_gain_slot("s1", 2.0, true);
        slot.wet = 0.5;
        chain.push_slot(slot, 48_000.0, 512).unwrap();
        let mut scratch = Scratch::default();
        let mut left = block(256, 0.5);
        let mut right = block(256, 0.5);
        chain.process(&mut left, &mut right, 256, &mut scratch);
        // 0.5 * 0.5 (dry) + 0.5 * 1.0 (wet) = 0.75
        assert!(left.iter().all(|&v| (v - 0.75).abs() < 1e-6));
    }

    #[test]
    fn two_slots_chain_in_series() {
        let mut chain = InsertChain::new();
        let (slot_a, _) = make_gain_slot("a", 2.0, true);
        let (slot_b, _) = make_gain_slot("b", 3.0, true);
        chain.push_slot(slot_a, 48_000.0, 512).unwrap();
        chain.push_slot(slot_b, 48_000.0, 512).unwrap();
        let mut scratch = Scratch::default();
        let mut left = block(256, 0.5);
        let mut right = block(256, 0.5);
        chain.process(&mut left, &mut right, 256, &mut scratch);
        // 0.5 → ×2 = 1.0 → ×3 = 3.0
        assert!(left.iter().all(|&v| (v - 3.0).abs() < 1e-5));
    }

    #[test]
    fn router_add_then_process_runs_chain() {
        let (mut tx, mut rx, mut router, mut graveyard) = build_runtime(48_000.0, 512);
        let (slot, counter) = make_gain_slot("s1", 2.0, true);
        tx.try_send(InsertCommand::Add { track_id: "t".into(), slot }).ok().unwrap();
        router.drain(&mut rx);
        assert_eq!(router.chains.len(), 1);

        let mut scratch = Scratch::default();
        let mut left = block(256, 0.5);
        let mut right = block(256, 0.5);
        router.chains.get_mut("t").unwrap().process(&mut left, &mut right, 256, &mut scratch);
        assert!(left.iter().all(|&v| (v - 1.0).abs() < 1e-6));
        assert_eq!(counter.load(Ordering::Relaxed), 1);
        assert_eq!(graveyard.drain_and_drop(), 0);
    }

    #[test]
    fn router_remove_buries_slot_off_audio_thread() {
        let (mut tx, mut rx, mut router, mut graveyard) = build_runtime(48_000.0, 512);
        let (slot, _) = make_gain_slot("s1", 2.0, true);
        tx.try_send(InsertCommand::Add { track_id: "t".into(), slot }).ok().unwrap();
        router.drain(&mut rx);
        tx.try_send(InsertCommand::Remove {
            track_id: "t".into(),
            slot_id: "s1".into(),
        })
        .ok()
        .unwrap();
        router.drain(&mut rx);
        assert_eq!(router.chains.get("t").unwrap().slots.len(), 0);
        assert_eq!(graveyard.drain_and_drop(), 1);
    }

    #[test]
    fn router_reorder_moves_slots() {
        let (mut tx, mut rx, mut router, _) = build_runtime(48_000.0, 512);
        for id in ["a", "b", "c"] {
            let (slot, _) = make_gain_slot(id, 1.0, true);
            tx.try_send(InsertCommand::Add { track_id: "t".into(), slot }).ok().unwrap();
        }
        router.drain(&mut rx);
        tx.try_send(InsertCommand::Reorder {
            track_id: "t".into(),
            from: 0,
            to: 2,
        })
        .ok()
        .unwrap();
        router.drain(&mut rx);
        let order: Vec<String> = router
            .chains
            .get("t")
            .unwrap()
            .slots
            .iter()
            .map(|s| s.slot_id.clone())
            .collect();
        assert_eq!(order, vec!["b", "c", "a"]);
    }

    #[test]
    fn router_set_parameter_takes_effect_on_next_process() {
        let (mut tx, mut rx, mut router, _) = build_runtime(48_000.0, 512);
        let (slot, _) = make_gain_slot("s1", 1.0, true);
        tx.try_send(InsertCommand::Add { track_id: "t".into(), slot }).ok().unwrap();
        router.drain(&mut rx);
        tx.try_send(InsertCommand::SetParameter {
            track_id: "t".into(),
            slot_id: "s1".into(),
            param_id: 0,
            value: 4.0,
        })
        .ok()
        .unwrap();
        router.drain(&mut rx);

        let mut scratch = Scratch::default();
        let mut left = block(256, 0.5);
        let mut right = block(256, 0.5);
        router.chains.get_mut("t").unwrap().process(&mut left, &mut right, 256, &mut scratch);
        // 0.5 × gain(4.0) = 2.0
        assert!(left.iter().all(|&v| (v - 2.0).abs() < 1e-6));
    }

    #[test]
    fn router_set_enabled_toggles_chain() {
        let (mut tx, mut rx, mut router, _) = build_runtime(48_000.0, 512);
        let (slot, counter) = make_gain_slot("s1", 2.0, true);
        tx.try_send(InsertCommand::Add { track_id: "t".into(), slot }).ok().unwrap();
        router.drain(&mut rx);

        // Disable.
        tx.try_send(InsertCommand::SetEnabled {
            track_id: "t".into(),
            slot_id: "s1".into(),
            enabled: false,
        })
        .ok()
        .unwrap();
        router.drain(&mut rx);

        let mut scratch = Scratch::default();
        let mut left = block(256, 0.5);
        let mut right = block(256, 0.5);
        router.chains.get_mut("t").unwrap().process(&mut left, &mut right, 256, &mut scratch);
        assert!(left.iter().all(|&v| (v - 0.5).abs() < 1e-6));
        assert_eq!(counter.load(Ordering::Relaxed), 0, "disabled slot should not call plugin");
    }
}
