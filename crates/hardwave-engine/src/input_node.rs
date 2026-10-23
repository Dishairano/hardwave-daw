//! InputNode — streams live audio from the input device into the graph.
//!
//! Holds the consumer side of a lock-free ring buffer that `hardwave-audio-io`'s
//! input callback pushes interleaved stereo samples into. On each process()
//! call the node drains one block worth of frames (or silence, when the ring
//! is empty or detached) and writes them to its stereo outputs, so armed
//! tracks hear themselves through their FX chain.

use parking_lot::Mutex;
use std::io::{Read, Seek, SeekFrom, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use crate::graph::{AudioNode, ProcessContext};

pub type SharedInputConsumer = Arc<Mutex<Option<rtrb::Consumer<f32>>>>;

/// Where a recording is written while it happens: straight to disk.
///
/// The audio thread writes into a small lock-free ring and never allocates
/// or waits; a writer thread empties the ring into a file as the take goes.
/// A take was held in memory before, in a buffer reserved for twenty
/// minutes (about 460 MB at 48 kHz) that a longer take simply ran out of.
/// Now it is as long as the disk allows, and the loop passes are cut from the
/// file by position when recording stops (`take_pass_ranges`), so even a long
/// take is never loaded back into memory.
///
/// `overflowed` reports a take that lost audio: the ring filled because the
/// disk could not keep up, or no file could be made. The caller tells the
/// user instead of handing them a file that looks complete.
pub struct CaptureTap {
    pub recording: AtomicBool,
    /// The audio thread's end of the ring. `try_lock` here only meets `arm`
    /// and `finish`, which run with recording off.
    producer: Mutex<Option<rtrb::Producer<f32>>>,
    /// The thread that writes the ring to disk, and where it writes.
    writer: Mutex<Option<TakeWriter>>,
    /// Folder the take's file is written in while recording.
    dir: Mutex<PathBuf>,
    /// Samples written so far by the audio thread.
    written: AtomicUsize,
    /// Set when audio was lost: the ring filled, or there was no file.
    overflowed: AtomicBool,
    /// Punch window in samples. The playlist has let people set a punch
    /// range, drawn it, and saved it with the project since it was written,
    /// and nothing read it: recording ignored the range completely.
    punch_enabled: AtomicBool,
    punch_in: AtomicU64,
    punch_out: AtomicU64,
    /// Where each loop pass starts in the capture, as a sample index.
    ///
    /// Recording with the loop on used to produce one long take holding
    /// every pass back to back, placed at the loop start: a four bar loop
    /// recorded three times became a twelve bar clip running eight bars past
    /// the loop. The tap now notes where the playhead jumped back, so the
    /// take can be split into one clip per pass. Fixed size so the audio
    /// thread never allocates; passes past the limit stay in the last one.
    pass_starts: [AtomicUsize; MAX_LOOP_PASSES],
    pass_count: AtomicUsize,
    /// Playhead at the previous block, to notice the jump back at a loop.
    last_block_start: AtomicU64,
    /// Round-trip latency in frames: how long after being played a sample
    /// reaches this tap.
    ///
    /// Takes used to be placed at the playhead the audio arrived at. By then
    /// the player had heard the song late by the output latency, and their
    /// playing had reached the DAW late by the input latency, so every take
    /// sat 10 to 30 ms behind the beat. Each sample is now placed at the
    /// position it was played at: the playhead minus this. The punch window
    /// and the loop split use that position too, so a punch-in cuts where
    /// the player crossed it rather than a few milliseconds later.
    latency: AtomicU64,
    /// Nothing played before this position is kept in the first pass: the
    /// first `latency` frames of a take were played before recording began.
    floor: AtomicU64,
    /// Position the next sample would be placed at while a loop jump is
    /// still working its way through the latency.
    eff_next: AtomicI64,
    /// Frames left that still belong to the pass before a loop jump.
    pending: AtomicU64,
    /// Where the first kept sample was placed, u64::MAX before one is.
    first_position: AtomicU64,
    /// Frames waiting in the input queue, smoothed. Part of the input
    /// latency the driver cannot report, measured on the audio thread.
    queue_frames: AtomicU32,
}

/// Loop passes one take can be split into. Beyond this the extra passes are
/// kept together in the last one rather than lost.
pub const MAX_LOOP_PASSES: usize = 64;

impl Default for CaptureTap {
    fn default() -> Self {
        Self {
            recording: AtomicBool::new(false),
            producer: Mutex::new(None),
            writer: Mutex::new(None),
            dir: Mutex::new(std::env::temp_dir().join("hardwave-takes")),
            written: AtomicUsize::new(0),
            overflowed: AtomicBool::new(false),
            punch_enabled: AtomicBool::new(false),
            punch_in: AtomicU64::new(0),
            punch_out: AtomicU64::new(u64::MAX),
            pass_starts: std::array::from_fn(|_| AtomicUsize::new(0)),
            pass_count: AtomicUsize::new(0),
            last_block_start: AtomicU64::new(0),
            latency: AtomicU64::new(0),
            floor: AtomicU64::new(0),
            eff_next: AtomicI64::new(0),
            pending: AtomicU64::new(0),
            first_position: AtomicU64::new(u64::MAX),
            queue_frames: AtomicU32::new(0),
        }
    }
}

impl CaptureTap {
    /// Start a take that streams to disk, with a ring of `seconds` between
    /// the audio thread and the disk.
    ///
    /// Called from the UI thread with recording off, so the ring, the file
    /// and the writer thread are made here rather than in the audio callback.
    pub fn arm(&self, sample_rate: u32, seconds: u32) {
        self.arm_compensated(sample_rate, seconds, 0, 0);
    }

    /// The folder takes are written in while recording: the song's own
    /// recordings folder, so a long take fills the disk the user chose for
    /// them rather than the system's temporary folder.
    pub fn set_dir(&self, dir: PathBuf) {
        *self.dir.lock() = dir;
    }

    /// Make the ring, the file and the writer thread for a new take.
    fn start_stream(&self, sample_rate: u32, seconds: u32) {
        // Whatever an earlier take left behind is finished first.
        drop(self.finish());
        let capacity = (sample_rate.max(1) as usize)
            .saturating_mul(seconds.max(1) as usize)
            .saturating_mul(2);
        let (producer, consumer) = rtrb::RingBuffer::new(capacity);
        let dir = self.dir.lock().clone();
        match TakeWriter::start(&dir, consumer) {
            Ok(writer) => {
                *self.writer.lock() = Some(writer);
                *self.producer.lock() = Some(producer);
            }
            Err(e) => {
                log::error!("recording: could not open a file in {}: {e}", dir.display());
                *self.producer.lock() = None;
            }
        }
    }

    /// Stop writing and hand over the take on disk: everything the audio
    /// thread wrote, once the writer has emptied the ring. None when no take
    /// was started. Call with recording off.
    pub fn finish(&self) -> Option<TakeFile> {
        let producer = self.producer.lock().take();
        let writer = self.writer.lock().take()?;
        drop(producer);
        let samples = self.written.swap(0, Ordering::Relaxed);
        match writer.finish() {
            Ok(path) => Some(TakeFile { path, samples }),
            Err(e) => {
                log::error!("recording: the take could not be written: {e}");
                self.overflowed.store(true, Ordering::Relaxed);
                None
            }
        }
    }

    /// Arm for a take whose samples reach the tap `latency` frames after
    /// they were played, starting at timeline position `floor`.
    pub fn arm_compensated(&self, sample_rate: u32, seconds: u32, latency: u64, floor: u64) {
        self.latency.store(latency, Ordering::Relaxed);
        self.floor.store(floor, Ordering::Relaxed);
        self.pending.store(0, Ordering::Relaxed);
        self.first_position.store(u64::MAX, Ordering::Relaxed);
        self.start_stream(sample_rate, seconds);
        self.written.store(0, Ordering::Relaxed);
        self.overflowed.store(false, Ordering::Relaxed);
        self.pass_count.store(0, Ordering::Relaxed);
        self.last_block_start.store(u64::MAX, Ordering::Relaxed);
    }

    /// Finish the take and say where each loop pass lies in its file, in
    /// samples, in the order they were played. One range when the loop never
    /// wrapped. Nothing is read back into memory.
    pub fn take_pass_ranges(&self) -> Option<(TakeFile, Vec<Range<usize>>)> {
        let count = self
            .pass_count
            .swap(0, Ordering::Relaxed)
            .min(MAX_LOOP_PASSES);
        let starts: Vec<usize> = (0..count)
            .map(|i| self.pass_starts[i].load(Ordering::Relaxed))
            .collect();
        let file = self.finish()?;
        let total = file.samples;
        let mut bounds: Vec<usize> = starts.into_iter().map(|b| b.min(total)).collect();
        bounds.retain(|&b| b > 0 && b < total);
        bounds.dedup();
        let mut ranges = Vec::with_capacity(bounds.len() + 1);
        let mut from = 0;
        for b in bounds {
            ranges.push(from..b);
            from = b;
        }
        ranges.push(from..total);
        ranges.retain(|r| !r.is_empty());
        Some((file, ranges))
    }

    /// The captured samples split at every loop jump, read back into memory.
    /// For short takes and tests; recording itself uses `take_pass_ranges`.
    pub fn take_passes(&self) -> Vec<Vec<f32>> {
        let Some((file, ranges)) = self.take_pass_ranges() else {
            return Vec::new();
        };
        ranges
            .into_iter()
            .map(|r| file.read(r).unwrap_or_default())
            .collect()
    }

    /// Take the captured samples, read back into memory, and reset for the
    /// next session.
    pub fn take(&self) -> Vec<f32> {
        self.pass_count.store(0, Ordering::Relaxed);
        self.finish()
            .map(|file| file.read(0..file.samples).unwrap_or_default())
            .unwrap_or_default()
    }

    /// Set the punch window, in samples on the project's timeline.
    ///
    /// With it enabled, only audio inside the window is captured, to the
    /// sample: a block that straddles punch-in keeps its tail, and one that
    /// straddles punch-out keeps its head.
    pub fn set_punch(&self, enabled: bool, in_samples: u64, out_samples: u64) {
        self.punch_enabled.store(enabled, Ordering::Relaxed);
        self.punch_in.store(in_samples, Ordering::Relaxed);
        self.punch_out
            .store(out_samples.max(in_samples), Ordering::Relaxed);
    }

    pub fn punch(&self) -> (bool, u64, u64) {
        (
            self.punch_enabled.load(Ordering::Relaxed),
            self.punch_in.load(Ordering::Relaxed),
            self.punch_out.load(Ordering::Relaxed),
        )
    }

    /// The part of a block that falls inside the punch window, as an offset
    /// and a length in frames. Pure, so the window arithmetic is testable
    /// without an audio device.
    fn punched_span(&self, block_start: u64, frames: usize) -> (usize, usize) {
        let (enabled, punch_in, punch_out) = self.punch();
        if !enabled {
            return (0, frames);
        }
        let block_end = block_start.saturating_add(frames as u64);
        if block_end <= punch_in || block_start >= punch_out {
            return (0, 0);
        }
        let start = punch_in.saturating_sub(block_start) as usize;
        let end = (punch_out.min(block_end) - block_start) as usize;
        (start.min(frames), end.min(frames).saturating_sub(start))
    }

    /// Timeline position of the first sample kept in the last take, or
    /// `None` when nothing was kept.
    pub fn first_position(&self) -> Option<u64> {
        match self.first_position.load(Ordering::Relaxed) {
            u64::MAX => None,
            p => Some(p),
        }
    }

    /// Frames the input queue holds on average. Audio thread writes it,
    /// the UI thread reads it when working out the latency of a take.
    pub fn queue_frames(&self) -> u32 {
        self.queue_frames.load(Ordering::Relaxed)
    }

    fn note_queue_frames(&self, frames: u32) {
        // Smoothed over about eight blocks: the depth swings by up to a
        // block depending on how the input and output callbacks interleave.
        let old = self.queue_frames.load(Ordering::Relaxed);
        let smoothed = if old == 0 {
            frames
        } else {
            (old * 7 + frames) / 8
        };
        self.queue_frames.store(smoothed, Ordering::Relaxed);
    }

    /// Whether the last session ran out of reserved room.
    pub fn overflowed(&self) -> bool {
        self.overflowed.load(Ordering::Relaxed)
    }

    /// Seconds captured so far, for a recording time read-out.
    pub fn seconds_captured(&self, sample_rate: u32) -> f64 {
        if sample_rate == 0 {
            return 0.0;
        }
        self.written.load(Ordering::Relaxed) as f64 / 2.0 / sample_rate as f64
    }

    /// Same as the audio thread's write, exposed for tests that stand in for
    /// `InputNode::process`. `block_start` is the playhead in samples.
    #[doc(hidden)]
    pub fn write_test_block_at(
        &self,
        left: &[f32],
        right: &[f32],
        frames: usize,
        block_start: u64,
    ) {
        self.write_block(left, right, frames, block_start)
    }

    #[doc(hidden)]
    pub fn write_test_block(&self, left: &[f32], right: &[f32], frames: usize) {
        self.write_block(left, right, frames, 0)
    }

    /// Write one block of interleaved stereo. Audio thread only.
    ///
    /// `block_start` is the playhead the block arrived at. Each sample is
    /// placed `latency` frames earlier, at the position it was played at.
    /// After a loop jump the first `latency` frames were still played in the
    /// pass before it, so they stay with that pass and the new pass starts
    /// once they are through.
    ///
    /// `try_lock` here can only fail against `arm` or `take`, both of which
    /// run with recording off, so in a live session this never contends and
    /// never drops a block.
    fn write_block(&self, left: &[f32], right: &[f32], frames: usize, block_start: u64) {
        let latency = self.latency.load(Ordering::Relaxed);
        let previous = self.last_block_start.swap(block_start, Ordering::Relaxed);
        if previous == u64::MAX {
            self.eff_next
                .store(block_start as i64 - latency as i64, Ordering::Relaxed);
            self.pending.store(0, Ordering::Relaxed);
        } else if block_start < previous {
            // The loop jumped. Checked before the punch window so a punched
            // loop still splits per pass.
            if self.pending.load(Ordering::Relaxed) > 0 {
                // A loop shorter than the latency: close the pass now.
                self.mark_pass();
            }
            if latency == 0 {
                self.mark_pass();
            } else {
                self.pending.store(latency, Ordering::Relaxed);
            }
        }

        let mut i = 0;
        while i < frames {
            let pending = self.pending.load(Ordering::Relaxed);
            if pending > 0 {
                let span = (pending as usize).min(frames - i);
                let eff = self.eff_next.load(Ordering::Relaxed);
                self.write_span(left, right, i, span, eff);
                i += span;
                let left_over = pending - span as u64;
                self.pending.store(left_over, Ordering::Relaxed);
                if left_over == 0 {
                    self.mark_pass();
                    self.eff_next.store(
                        block_start as i64 + i as i64 - latency as i64,
                        Ordering::Relaxed,
                    );
                } else {
                    self.eff_next.store(eff + span as i64, Ordering::Relaxed);
                }
            } else {
                let span = frames - i;
                let eff = block_start as i64 + i as i64 - latency as i64;
                self.write_span(left, right, i, span, eff);
                self.eff_next.store(eff + span as i64, Ordering::Relaxed);
                i = frames;
            }
        }
    }

    /// Note that the samples written from here on belong to a new pass.
    fn mark_pass(&self) {
        let n = self.pass_count.load(Ordering::Relaxed);
        if n < MAX_LOOP_PASSES {
            self.pass_starts[n].store(self.written.load(Ordering::Relaxed), Ordering::Relaxed);
            self.pass_count.store(n + 1, Ordering::Relaxed);
        }
    }

    /// Write `len` frames starting at `offset` in the block, the first of
    /// which was played at timeline position `eff`.
    fn write_span(&self, left: &[f32], right: &[f32], offset: usize, len: usize, eff: i64) {
        // Before the first loop jump nothing earlier than the record start
        // is kept; after one, nothing before the start of the song.
        let floor = if self.pass_count.load(Ordering::Relaxed) == 0 {
            self.floor.load(Ordering::Relaxed) as i64
        } else {
            0
        };
        let skip = (floor - eff).clamp(0, len as i64) as usize;
        let len = len - skip;
        if len == 0 {
            return;
        }
        let pos = (eff + skip as i64) as u64;
        let (punch_offset, take) = self.punched_span(pos, len);
        if take == 0 {
            return;
        }
        let from = offset + skip + punch_offset;
        let Some(mut guard) = self.producer.try_lock() else {
            self.overflowed.store(true, Ordering::Relaxed);
            return;
        };
        let Some(producer) = guard.as_mut() else {
            self.overflowed.store(true, Ordering::Relaxed);
            return;
        };
        let start = self.written.load(Ordering::Relaxed);
        if start == 0 {
            self.first_position
                .store(pos + punch_offset as u64, Ordering::Relaxed);
        }
        // The ring is only full when the disk fell behind.
        let n = take.min(producer.slots() / 2);
        if n < take {
            self.overflowed.store(true, Ordering::Relaxed);
        }
        if n == 0 {
            return;
        }
        let Ok(mut chunk) = producer.write_chunk(n * 2) else {
            self.overflowed.store(true, Ordering::Relaxed);
            return;
        };
        let (a, b) = chunk.as_mut_slices();
        for (k, slot) in a.iter_mut().chain(b.iter_mut()).enumerate() {
            let i = from + k / 2;
            *slot = if k % 2 == 0 { left[i] } else { right[i] };
        }
        chunk.commit_all();
        self.written.store(start + n * 2, Ordering::Relaxed);
    }
}

/// The thread that empties a take's ring into its file.
struct TakeWriter {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    handle: std::thread::JoinHandle<std::io::Result<()>>,
}

/// How many takes this run has started, for unique file names.
static TAKE_COUNTER: AtomicU64 = AtomicU64::new(0);

impl TakeWriter {
    fn start(dir: &Path, mut consumer: rtrb::Consumer<f32>) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let n = TAKE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!(".recording-{}-{n}.f32", std::process::id()));
        let file = std::fs::File::create(&path)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let handle = std::thread::Builder::new()
            .name("take-writer".into())
            .spawn(move || {
                let mut out = std::io::BufWriter::with_capacity(1 << 18, file);
                loop {
                    let ready = consumer.slots();
                    if ready > 0 {
                        let chunk = consumer
                            .read_chunk(ready)
                            .map_err(|e| std::io::Error::other(e.to_string()))?;
                        let (a, b) = chunk.as_slices();
                        for s in a.iter().chain(b.iter()) {
                            out.write_all(&s.to_le_bytes())?;
                        }
                        chunk.commit_all();
                    } else if stopping.load(Ordering::Acquire) {
                        break;
                    } else {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                }
                out.flush()
            })?;
        Ok(Self { path, stop, handle })
    }

    /// Write out what is left in the ring and close the file.
    fn finish(self) -> std::io::Result<PathBuf> {
        self.stop.store(true, Ordering::Release);
        match self.handle.join() {
            Ok(result) => result.map(|()| self.path),
            Err(_) => Err(std::io::Error::other("the take writer stopped")),
        }
    }
}

/// A finished take on disk: interleaved stereo f32 samples, `samples` long.
/// The file is removed when this is dropped, once its passes are written out.
pub struct TakeFile {
    pub path: PathBuf,
    pub samples: usize,
}

impl TakeFile {
    /// Samples `range` of the take.
    pub fn read(&self, range: Range<usize>) -> std::io::Result<Vec<f32>> {
        let range = range.start.min(self.samples)..range.end.min(self.samples);
        let mut file = std::fs::File::open(&self.path)?;
        file.seek(SeekFrom::Start(range.start as u64 * 4))?;
        let mut bytes = vec![0u8; range.len() * 4];
        file.read_exact(&mut bytes)?;
        Ok(bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect())
    }
}

impl Drop for TakeFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub struct InputNode {
    consumer: SharedInputConsumer,
    capture: Option<Arc<CaptureTap>>,
}

impl InputNode {
    pub fn new(consumer: SharedInputConsumer) -> Self {
        Self {
            consumer,
            capture: None,
        }
    }

    /// Attach a capture tap. When the tap's `recording` flag is true and
    /// the input ring is producing samples, every block the node drains
    /// is also pushed to the tap's buffer. Detach by calling
    /// `set_capture(None)`.
    pub fn set_capture(&mut self, capture: Option<Arc<CaptureTap>>) {
        self.capture = capture;
    }
}

/// Records another node's output: a bus or the master, for resampling. It
/// sits after that node with nothing after it, and writes what reaches it
/// to the recording while recording is on.
pub struct RecordTapNode {
    capture: Arc<CaptureTap>,
}

impl RecordTapNode {
    pub fn new(capture: Arc<CaptureTap>) -> Self {
        Self { capture }
    }
}

impl AudioNode for RecordTapNode {
    fn name(&self) -> &str {
        "Record tap"
    }

    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [Vec<f32>],
        _midi_in: &[hardwave_midi::MidiEvent],
        _midi_out: &mut Vec<hardwave_midi::MidiEvent>,
        ctx: &ProcessContext,
    ) {
        for ch in outputs.iter_mut() {
            ch.fill(0.0);
        }
        if !self.capture.recording.load(Ordering::Relaxed) {
            return;
        }
        let (Some(left), Some(right)) = (inputs.first(), inputs.get(1)) else {
            return;
        };
        let n = left.len().min(right.len()).min(ctx.buffer_size as usize);
        self.capture
            .write_block(left, right, n, ctx.position_samples);
    }
}

impl AudioNode for InputNode {
    fn name(&self) -> &str {
        "Input"
    }

    fn process(
        &mut self,
        _inputs: &[&[f32]],
        outputs: &mut [Vec<f32>],
        _midi_in: &[hardwave_midi::MidiEvent],
        _midi_out: &mut Vec<hardwave_midi::MidiEvent>,
        ctx: &ProcessContext,
    ) {
        let buf_size = outputs.first().map(|o| o.len()).unwrap_or(0);
        for ch in outputs.iter_mut() {
            ch.fill(0.0);
        }
        if buf_size == 0 || outputs.len() < 2 {
            return;
        }

        // try_lock so a UI-thread rebuild that's swapping the consumer can
        // never stall the audio callback. If we can't lock (or no consumer
        // is attached), the outputs stay at silence for this block.
        let mut guard = match self.consumer.try_lock() {
            Some(g) => g,
            None => return,
        };
        let cons = match guard.as_mut() {
            Some(c) => c,
            None => return,
        };

        if let Some(cap) = &self.capture {
            // Stereo samples waiting, as frames: how long the oldest one has
            // been queued before this block reads it.
            cap.note_queue_frames((cons.slots() / 2) as u32);
        }

        let (left, rest) = outputs.split_at_mut(1);
        let (right, _) = rest.split_at_mut(1);
        for (l_out, r_out) in left[0].iter_mut().zip(right[0].iter_mut()).take(buf_size) {
            *l_out = cons.pop().unwrap_or(0.0);
            *r_out = cons.pop().unwrap_or(0.0);
        }

        // Recording tap: the block just produced goes into the capture
        // ring, interleaved L, R, L, R, and from there to the take's file.
        if let Some(cap) = &self.capture {
            if cap.recording.load(Ordering::Relaxed) {
                cap.write_block(&left[0], &right[0], buf_size, ctx.position_samples);
            }
        }
    }
}

#[cfg(test)]
mod capture_tests {
    use super::*;

    fn tap(seconds: u32) -> CaptureTap {
        let t = CaptureTap::default();
        t.arm(48_000, seconds);
        t.recording.store(true, Ordering::Relaxed);
        t
    }

    #[test]
    fn a_block_is_captured_interleaved() {
        let t = tap(1);
        t.write_test_block(&[0.1, 0.2], &[-0.1, -0.2], 2);
        let out = t.take();
        assert_eq!(out, vec![0.1, -0.1, 0.2, -0.2]);
    }

    #[test]
    fn consecutive_blocks_keep_their_order() {
        let t = tap(1);
        t.write_test_block(&[0.1], &[0.1], 1);
        t.write_test_block(&[0.2], &[0.2], 1);
        t.write_test_block(&[0.3], &[0.3], 1);
        let out = t.take();
        assert_eq!(out.len(), 6);
        assert_eq!(out[0], 0.1);
        assert_eq!(out[2], 0.2);
        assert_eq!(out[4], 0.3);
    }

    /// A take used to be cut off at the memory reserved for it. It now goes
    /// to disk while it is played, so a take far longer than the ring
    /// between the audio thread and the disk arrives whole.
    #[test]
    fn a_take_longer_than_the_ring_goes_to_disk_whole() {
        let t = tap(1);
        // Three seconds through a one-second ring, at about the pace audio
        // arrives, so the writer keeps up as it does on a real disk.
        for _ in 0..(48_000 / 480 * 3) {
            t.write_test_block(&[0.4; 480], &[0.4; 480], 480);
            std::thread::sleep(std::time::Duration::from_micros(500));
        }
        assert!(!t.overflowed(), "nothing may be lost");
        let out = t.take();
        assert_eq!(out.len(), 48_000 * 3 * 2);
        assert!(out.iter().all(|&s| s == 0.4));
    }

    /// A folder that cannot be made, for a take with nowhere to go.
    fn unwritable_dir() -> PathBuf {
        let file = std::env::temp_dir().join(format!("hw-not-a-folder-{}", std::process::id()));
        std::fs::write(&file, b"x").unwrap();
        file.join("takes")
    }

    #[test]
    fn a_take_with_nowhere_to_go_is_reported_not_lost_quietly() {
        let t = CaptureTap::default();
        t.set_dir(unwritable_dir());
        t.arm(48_000, 1);
        t.recording.store(true, Ordering::Relaxed);
        t.write_test_block(&[0.4; 480], &[0.4; 480], 480);
        assert!(
            t.overflowed(),
            "a take that was not written must be reported"
        );
        assert!(t.take().is_empty());
    }

    #[test]
    fn taking_a_session_resets_for_the_next_one() {
        let t = tap(1);
        t.write_test_block(&[0.1, 0.2], &[0.1, 0.2], 2);
        assert_eq!(t.take().len(), 4);
        assert!(t.take().is_empty(), "a second take must be empty");

        // The next take arms again, as the engine does for every take.
        t.arm(48_000, 1);
        t.write_test_block(&[0.9], &[0.9], 1);
        assert_eq!(t.take(), vec![0.9, 0.9], "and the tap still works after");
    }

    #[test]
    fn the_time_read_out_follows_what_was_captured() {
        let t = tap(2);
        t.write_test_block(&[0.1; 24_000], &[0.1; 24_000], 24_000);
        assert!((t.seconds_captured(48_000) - 0.5).abs() < 1e-9);
    }

    /// The punch range was drawn in the playlist and saved with the project,
    /// and recording ignored it: it captured the whole pass.
    #[test]
    fn with_punch_off_the_whole_block_is_captured() {
        let t = tap(4);
        t.write_test_block_at(&[0.5; 480], &[0.5; 480], 480, 96_000);
        assert_eq!(t.take().len(), 960);
    }

    #[test]
    fn a_block_before_punch_in_is_not_captured() {
        let t = tap(4);
        t.set_punch(true, 48_000, 96_000);
        t.write_test_block_at(&[0.5; 480], &[0.5; 480], 480, 0);
        assert!(t.take().is_empty(), "audio before the punch point got in");
    }

    #[test]
    fn a_block_after_punch_out_is_not_captured() {
        let t = tap(4);
        t.set_punch(true, 48_000, 96_000);
        t.write_test_block_at(&[0.5; 480], &[0.5; 480], 480, 96_000);
        assert!(t.take().is_empty(), "audio after the punch point got in");
    }

    #[test]
    fn a_block_straddling_punch_in_keeps_only_its_tail() {
        let t = tap(4);
        // Punch in 100 frames into this block.
        t.set_punch(true, 48_100, 96_000);
        let left: Vec<f32> = (0..480).map(|i| i as f32).collect();
        t.write_test_block_at(&left, &left, 480, 48_000);
        let out = t.take();
        assert_eq!(
            out.len(),
            (480 - 100) * 2,
            "kept the wrong number of frames"
        );
        assert_eq!(out[0], 100.0, "the first captured frame is the punch point");
    }

    #[test]
    fn a_block_straddling_punch_out_keeps_only_its_head() {
        let t = tap(4);
        t.set_punch(true, 48_000, 48_200);
        let left: Vec<f32> = (0..480).map(|i| i as f32).collect();
        t.write_test_block_at(&left, &left, 480, 48_000);
        let out = t.take();
        assert_eq!(out.len(), 200 * 2);
        assert_eq!(out[0], 0.0);
        assert_eq!(out[out.len() - 2], 199.0, "cut exactly at the punch point");
    }

    #[test]
    fn a_punch_window_inside_one_block_keeps_only_that_slice() {
        let t = tap(4);
        t.set_punch(true, 48_100, 48_150);
        let left: Vec<f32> = (0..480).map(|i| i as f32).collect();
        t.write_test_block_at(&left, &left, 480, 48_000);
        let out = t.take();
        assert_eq!(out.len(), 50 * 2);
        assert_eq!(out[0], 100.0);
    }

    #[test]
    fn a_reversed_punch_range_is_treated_as_empty_not_infinite() {
        let t = tap(4);
        // Out before in: clamped so the window is zero-length, not the whole
        // song, which would silently record everything.
        t.set_punch(true, 96_000, 48_000);
        let (_, punch_in, punch_out) = t.punch();
        assert_eq!(punch_in, 96_000);
        assert_eq!(punch_out, 96_000);
        t.write_test_block_at(&[0.5; 480], &[0.5; 480], 480, 96_000);
        assert!(t.take().is_empty());
    }

    /// Recording with the loop on used to hand back one take holding every
    /// pass back to back.
    #[test]
    fn a_looped_take_splits_into_one_pass_per_loop() {
        let t = tap(4);
        // Three passes over a loop from 48000 to 48960, in 480-frame blocks.
        for pass in 0..3 {
            t.write_test_block_at(&[pass as f32; 480], &[pass as f32; 480], 480, 48_000);
            t.write_test_block_at(&[pass as f32; 480], &[pass as f32; 480], 480, 48_480);
        }
        let passes = t.take_passes();
        assert_eq!(passes.len(), 3, "three passes, three takes");
        for (i, p) in passes.iter().enumerate() {
            assert_eq!(p.len(), 960 * 2, "pass {i} has the wrong length");
            assert_eq!(p[0], i as f32, "pass {i} holds another pass's audio");
        }
    }

    #[test]
    fn a_take_without_a_loop_is_one_pass() {
        let t = tap(4);
        t.write_test_block_at(&[0.1; 480], &[0.1; 480], 480, 0);
        t.write_test_block_at(&[0.1; 480], &[0.1; 480], 480, 480);
        assert_eq!(t.take_passes().len(), 1);
    }

    #[test]
    fn a_punched_loop_still_splits_per_pass() {
        let t = tap(4);
        // Loop 48000..49920, punch only the middle 960 frames.
        t.set_punch(true, 48_480, 49_440);
        for _ in 0..2 {
            for b in 0..4 {
                let at = 48_000 + b * 480;
                t.write_test_block_at(&[0.3; 480], &[0.3; 480], 480, at);
            }
        }
        let passes = t.take_passes();
        assert_eq!(passes.len(), 2);
        assert!(
            passes.iter().all(|p| p.len() == 960 * 2),
            "each pass is the punch window"
        );
    }

    #[test]
    fn a_new_take_forgets_the_last_takes_passes() {
        let t = tap(4);
        t.write_test_block_at(&[0.1; 480], &[0.1; 480], 480, 960);
        t.write_test_block_at(&[0.1; 480], &[0.1; 480], 480, 0);
        assert_eq!(t.take_passes().len(), 2);

        t.arm(48_000, 4);
        t.write_test_block_at(&[0.2; 480], &[0.2; 480], 480, 0);
        assert_eq!(
            t.take_passes().len(),
            1,
            "passes carried over from the last take"
        );
    }

    #[test]
    fn arming_clears_the_previous_overflow() {
        let t = CaptureTap::default();
        t.set_dir(unwritable_dir());
        t.arm(48_000, 1);
        t.write_test_block(&[0.4; 480], &[0.4; 480], 480);
        assert!(t.overflowed());
        t.set_dir(std::env::temp_dir().join("hardwave-takes"));
        t.arm(48_000, 1);
        assert!(!t.overflowed(), "a new take starts clean");
    }

    fn compensated(latency: u64, floor: u64) -> CaptureTap {
        let t = CaptureTap::default();
        t.arm_compensated(48_000, 1, latency, floor);
        t.recording.store(true, Ordering::Relaxed);
        t
    }

    fn ramp(from: usize, n: usize) -> Vec<f32> {
        (from..from + n).map(|i| i as f32).collect()
    }

    #[test]
    fn a_sample_is_placed_where_it_was_played_not_where_it_arrived() {
        // Recording starts at 10. The first block arrives at 10 but was
        // played 4 frames earlier, before recording began, so its first 4
        // frames are dropped and the take starts with the fifth, at 10.
        let t = compensated(4, 10);
        let block = ramp(0, 8);
        t.write_test_block_at(&block, &block, 8, 10);
        assert_eq!(t.first_position(), Some(10));
        let out = t.take();
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        assert_eq!(left, vec![4.0, 5.0, 6.0, 7.0]);
    }

    #[test]
    fn punch_in_cuts_where_it_was_played() {
        // Punch in at 20 with 4 frames of latency: the block arriving at 22
        // was played from 18, so only its last two frames (played at 20 and
        // 21) are inside the window.
        let t = compensated(4, 0);
        t.set_punch(true, 20, 1_000);
        let block = ramp(0, 4);
        t.write_test_block_at(&block, &block, 4, 22);
        assert_eq!(t.first_position(), Some(20));
        let out = t.take();
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        assert_eq!(left, vec![2.0, 3.0]);
    }

    #[test]
    fn after_a_loop_jump_the_late_frames_stay_in_the_pass_they_were_played_in() {
        // Three frames of latency. When the playhead jumps from 108 back to
        // 0, the next three frames were still played at the end of the first
        // pass; only the fourth belongs to the new one.
        let t = compensated(3, 0);
        t.write_test_block_at(&ramp(0, 4), &ramp(0, 4), 4, 100);
        t.write_test_block_at(&ramp(4, 4), &ramp(4, 4), 4, 104);
        t.write_test_block_at(&ramp(8, 4), &ramp(8, 4), 4, 0);
        assert_eq!(t.first_position(), Some(97));
        let passes = t.take_passes();
        assert_eq!(passes.len(), 2);
        let first: Vec<f32> = passes[0].iter().step_by(2).copied().collect();
        let second: Vec<f32> = passes[1].iter().step_by(2).copied().collect();
        assert_eq!(first, ramp(0, 11), "8 frames before the jump and 3 after");
        assert_eq!(second, vec![11.0]);
    }

    #[test]
    fn a_later_pass_is_not_cut_by_the_first_passes_record_start() {
        // Recording began at 50 inside a loop from 0 to 100. The second pass
        // starts at 0, before the record start, and must be kept whole.
        let t = compensated(2, 50);
        t.write_test_block_at(&ramp(0, 4), &ramp(0, 4), 4, 96);
        t.write_test_block_at(&ramp(4, 4), &ramp(4, 4), 4, 0);
        t.write_test_block_at(&ramp(8, 4), &ramp(8, 4), 4, 4);
        let passes = t.take_passes();
        assert_eq!(passes.len(), 2);
        let second: Vec<f32> = passes[1].iter().step_by(2).copied().collect();
        assert_eq!(second, ramp(6, 6));
    }

    #[test]
    fn without_latency_placement_is_unchanged() {
        let t = compensated(0, 0);
        let block = ramp(0, 4);
        t.write_test_block_at(&block, &block, 4, 40);
        assert_eq!(t.first_position(), Some(40));
        assert_eq!(t.take().len(), 8);
    }
}
