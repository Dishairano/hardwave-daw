//! MasterNode — sums all track inputs, runs the master insert chain, and
//! applies master volume.
//!
//! Signal order mirrors a track: sum → inserts → fader. That means a limiter
//! or EQ on the master processes the full mix and is *not* affected by the
//! master fader, which is what a producer expects when they pull the master
//! down to check headroom.

use crate::graph::{AudioNode, ProcessContext};
use atomic_float::AtomicF64;
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub struct MasterNode {
    /// dB value read from the shared transport state each process block.
    /// Using an atomic keeps volume changes lock-free and graph-rebuild-free.
    volume_db: Arc<AtomicF64>,
    /// Master insert chain. The Master track is not audio-bearing (it gets no
    /// `TrackNode`), so before this existed a plug-in added to the master
    /// strip was persisted into the project and then silently did nothing —
    /// its `InsertCommand` had no node to route to, in playback or export.
    chain: crate::insert_chain::InsertChain,
    chain_scratch: crate::insert_chain::Scratch,
    /// Id of the project's Master track, when it has one. Reporting it from
    /// `track_id()` is what lets the rebuild stash and restore this chain like
    /// any other — without it the master's plug-ins would be dropped every
    /// time the graph rebuilt.
    track_id: Option<String>,
}

impl MasterNode {
    pub fn new(volume_db: Arc<AtomicF64>, track_id: Option<String>) -> Self {
        Self {
            volume_db,
            chain: crate::insert_chain::InsertChain::new(),
            chain_scratch: crate::insert_chain::Scratch::default(),
            track_id,
        }
    }
}

impl AudioNode for MasterNode {
    fn name(&self) -> &str {
        "Master"
    }

    fn track_id(&self) -> Option<&str> {
        self.track_id.as_deref()
    }

    fn snapshot_plugin_states(&self) -> Vec<(String, Vec<u8>)> {
        self.chain
            .slots
            .iter()
            .map(|slot| (slot.slot_id.clone(), slot.plugin.get_state()))
            .collect()
    }

    fn apply_insert_command(
        &mut self,
        cmd: crate::insert_chain::InsertCommand,
        graveyard: &mut crate::insert_chain::PluginGraveyardSender,
        sample_rate: f64,
        max_block_size: u32,
    ) {
        use crate::insert_chain::InsertCommand;
        match cmd {
            InsertCommand::Add { slot, .. } => {
                if let Err(e) = self.chain.push_slot(slot, sample_rate, max_block_size) {
                    log::warn!("master insert add failed: {e}");
                }
            }
            InsertCommand::Remove { slot_id, .. } => {
                if let Some(slot) = self.chain.take_slot(&slot_id) {
                    if graveyard.try_bury(slot).is_err() {
                        log::warn!(
                            "master graveyard full while removing {slot_id}; slot will leak until engine teardown"
                        );
                    }
                }
            }
            InsertCommand::Reorder { from, to, .. } => self.chain.reorder(from, to),
            InsertCommand::SetEnabled {
                slot_id, enabled, ..
            } => {
                self.chain.set_enabled(&slot_id, enabled);
            }
            InsertCommand::SetWet { slot_id, wet, .. } => {
                self.chain.set_wet(&slot_id, wet);
            }
            InsertCommand::SetParameter {
                slot_id,
                param_id,
                value,
                ..
            } => {
                self.chain.set_parameter(&slot_id, param_id, value);
            }
            InsertCommand::SetState { slot_id, bytes, .. } => {
                self.chain.set_state(&slot_id, &bytes);
            }
        }
    }

    /// Build-time insert population for offline render, so an export applies
    /// the master chain exactly as playback does.
    fn push_offline_slot(
        &mut self,
        slot: crate::insert_chain::LiveSlot,
        sample_rate: f64,
        max_block_size: u32,
    ) {
        if let Err(e) = self.chain.push_slot(slot, sample_rate, max_block_size) {
            log::warn!("master offline insert add failed: {e}");
        }
    }

    fn take_chain(&mut self) -> Option<crate::insert_chain::InsertChain> {
        let taken = std::mem::take(&mut self.chain);
        if taken.slots.is_empty() {
            None
        } else {
            Some(taken)
        }
    }

    fn restore_chain(&mut self, chain: crate::insert_chain::InsertChain) {
        self.chain = chain;
    }

    fn set_slot_sidechain(&mut self, slot_id: &str, active: bool) {
        self.chain.set_slot_sidechain(slot_id, active);
    }

    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [Vec<f32>],
        _midi_in: &[hardwave_midi::MidiEvent],
        _midi_out: &mut Vec<hardwave_midi::MidiEvent>,
        ctx: &ProcessContext,
    ) {
        let buf_size = ctx.buffer_size as usize;
        if outputs.len() < 2 {
            return;
        }
        outputs[0].resize(buf_size, 0.0);
        outputs[1].resize(buf_size, 0.0);

        // 1. Sum the incoming mix.
        for (i, sample) in outputs[0].iter_mut().enumerate() {
            *sample = inputs.first().and_then(|ch| ch.get(i)).copied().unwrap_or(0.0);
        }
        for (i, sample) in outputs[1].iter_mut().enumerate() {
            *sample = inputs.get(1).and_then(|ch| ch.get(i)).copied().unwrap_or(0.0);
        }

        // 2. Master insert chain, pre-fader. Split the borrow so the chain can
        //    take both channels mutably at once.
        if !self.chain.slots.is_empty() {
            let (left, right) = outputs.split_at_mut(1);
            self.chain.process(
                &mut left[0],
                &mut right[0],
                buf_size,
                &mut self.chain_scratch,
                &[],
                None,
            );
        }

        // 3. Master fader.
        let db = self.volume_db.load(Ordering::Relaxed);
        let gain = if db <= -100.0 {
            0.0
        } else {
            10.0_f64.powf(db / 20.0) as f32
        };
        if gain != 1.0 {
            for sample in outputs[0].iter_mut() {
                *sample *= gain;
            }
            for sample in outputs[1].iter_mut() {
                *sample *= gain;
            }
        }
    }
}
