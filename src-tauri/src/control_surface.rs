//! Control surfaces: a desk with real faders driving the mixer, speaking
//! Mackie Control (MCU) or HUI.
//!
//! A control surface speaks ordinary MIDI, but not the MIDI Learn kind:
//! the faders are pitch bend, one channel each, and the buttons are
//! notes at fixed numbers. Learn could map one knob to one parameter;
//! it could never give you eight faders, their mutes and their solos
//! at once, and it could not move a motorised fader back.
//!
//! Eight faders, mute, solo and arm per strip, the V-pots (pan), the
//! transport keys, and bank left and right come in. Back to the desk, a
//! few times a second and only what changed ([`frame_messages`]): the
//! motorised faders, the mute, solo and arm lights, the V-pot rings, and
//! each track's name and level on the scribble strips. A fader with a
//! hand on it (the desk says so with its touch notes) is left alone, so
//! the motor never fights the hand.
//!
//! HUI carries the same things in other messages, all controllers on the
//! first channel: a button is a zone (a row of up to eight) and then a
//! port in it, a fader its high bits and then its low ones. Its scribble
//! strips hold four characters, so they show the name only. A HUI desk
//! counts itself off line unless the host pings it once a second
//! ([`HUI_PING`]).

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hardwave_midi::MidiEvent;
use hardwave_project::Project;

/// Faders on a desk, and the master beside them.
const STRIPS: usize = 8;

/// Note numbers every Mackie-compatible desk sends.
const NOTE_REC_FIRST: u8 = 0; // 0..7   arm
const NOTE_SOLO_FIRST: u8 = 8; // 8..15  solo
const NOTE_MUTE_FIRST: u8 = 16; // 16..23 mute
const NOTE_BANK_LEFT: u8 = 46;
const NOTE_BANK_RIGHT: u8 = 47;
const NOTE_REWIND: u8 = 91;
const NOTE_FORWARD: u8 = 92;
const NOTE_STOP: u8 = 93;
const NOTE_PLAY: u8 = 94;
const NOTE_RECORD: u8 = 95;
const NOTE_VPOT_PUSH_FIRST: u8 = 32; // 32..39 V-pot pressed
const NOTE_TOUCH_FIRST: u8 = 104; // 104..111 a hand on the fader
const CC_VPOT_FIRST: u8 = 16; // 16..23 V-pot turned, relative
const CC_RING_FIRST: u8 = 48; // 48..55 V-pot LED ring

/// Controller numbers a HUI desk uses, all on the first channel.
const HUI_CC_FADER_HIGH: u8 = 0x00; // 0x00..0x07, and 0x08 for a master (iCON desks)
const HUI_CC_FADER_HIGH_LAST: u8 = 0x08;
const HUI_CC_FADER_LOW: u8 = 0x20; // 0x20..0x28, the low bits after the high
const HUI_CC_FADER_LOW_LAST: u8 = 0x28;
const HUI_CC_ZONE_OUT: u8 = 0x0C; // which row of lights the next message is for
const HUI_CC_ZONE_IN: u8 = 0x0F; // which row the next button is in
const HUI_CC_RING_FIRST: u8 = 0x10; // 0x10..0x17 V-pot LED ring
const HUI_CC_PORT_OUT: u8 = 0x2C; // a light in that row on or off
const HUI_CC_PORT_IN: u8 = 0x2F; // a button in that row down or up
const HUI_CC_VPOT_FIRST: u8 = 0x40; // 0x40..0x47 V-pot turned, relative
const HUI_CC_VPOT_LAST: u8 = 0x47;
/// Zones 0..7 are the strips; in each, a port per button.
const HUI_PORT_TOUCH: u8 = 0;
const HUI_PORT_MUTE: u8 = 2;
const HUI_PORT_SOLO: u8 = 3;
const HUI_PORT_VPOT_PUSH: u8 = 5;
const HUI_PORT_ARM: u8 = 7;
const HUI_ZONE_BANK: u8 = 0x0A; // port 1 bank left, 3 bank right
const HUI_ZONE_TRANSPORT: u8 = 0x0E; // 1 rewind, 2 forward, 3 stop, 4 play, 5 record

/// What the host sends a HUI desk once a second so it stays on line. The
/// desk answers with the same note at full velocity.
pub const HUI_PING: [u8; 3] = [0x90, 0x00, 0x00];
/// A ping answered after this long without one is a desk that was
/// switched off or unplugged, and shows nothing until it is sent again.
const HUI_SILENCE: Duration = Duration::from_secs(3);

/// Which language the desk speaks. Both carry the same things: eight
/// faders with their buttons and lights, the V-pots, the transport keys
/// and the banks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum DeskProtocol {
    #[default]
    Mackie = 0,
    Hui = 1,
}

/// What the desk is doing: whether it is listened to at all, and which
/// eight tracks its strips are on.
#[derive(Default)]
pub struct ControlSurface {
    pub enabled: AtomicBool,
    /// Index of the first track under the strips.
    pub bank: AtomicUsize,
    /// Faders with a hand on them, one bit per strip.
    pub touched: std::sync::atomic::AtomicU8,
    /// What the desk was last sent; none sends everything next time.
    pub shown: std::sync::Mutex<Option<DeskFrame>>,
    /// The desk's [`DeskProtocol`], as its number.
    protocol: AtomicU8,
}

impl ControlSurface {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Send the whole desk next time: a new bank, the desk just switched
    /// on, or plugged in again.
    pub fn redraw(&self) {
        if let Ok(mut shown) = self.shown.lock() {
            *shown = None;
        }
    }

    pub fn protocol(&self) -> DeskProtocol {
        if self.protocol.load(Ordering::Relaxed) == DeskProtocol::Hui as u8 {
            DeskProtocol::Hui
        } else {
            DeskProtocol::Mackie
        }
    }

    /// Speak another protocol: the desk is sent everything again in it,
    /// and no fader counts as held until the new desk says so.
    pub fn set_protocol(&self, protocol: DeskProtocol) {
        self.protocol.store(protocol as u8, Ordering::Relaxed);
        self.touched.store(0, Ordering::Relaxed);
        self.redraw();
    }

    pub fn set_touched(&self, strip: usize, on: bool) {
        let bit = 1u8 << (strip & 7);
        if on {
            self.touched.fetch_or(bit, Ordering::Relaxed);
        } else {
            self.touched.fetch_and(!bit, Ordering::Relaxed);
        }
    }
}

/// What one message from the desk asks for.
///
/// Worked out without touching the engine so it can be tested on its
/// own: the desk's protocol is the part that is easy to get wrong.
#[derive(Debug, Clone, PartialEq)]
pub enum SurfaceAction {
    Fader {
        strip: usize,
        value: f32,
    },
    MasterFader {
        value: f32,
    },
    Mute {
        strip: usize,
    },
    Solo {
        strip: usize,
    },
    Arm {
        strip: usize,
    },
    BankLeft,
    BankRight,
    Play,
    Stop,
    Record,
    Rewind,
    Forward,
    /// Set the tempo. No desk sends this, but a phone running TouchOSC
    /// does, and it is the same kind of message.
    Tempo {
        bpm: f64,
    },
    /// A V-pot turned: clicks, positive clockwise.
    VPot {
        strip: usize,
        clicks: i32,
    },
    /// A V-pot pressed: pan back to the centre.
    VPotPush {
        strip: usize,
    },
    /// A hand on (or off) a touch-sensitive fader.
    Touch {
        strip: usize,
        on: bool,
    },
    Pan {
        strip: usize,
        value: f32,
    },
    /// Jump the playhead, in beats from the start.
    Goto {
        beats: f64,
    },
    /// The desk is back (a HUI answering the ping after a silence) and
    /// shows nothing: send it everything.
    Hello,
}

/// Read one MIDI message as a desk message, or nothing when it is not
/// one. Note-offs are ignored: a desk sends a pair per button press and
/// acting on both would toggle twice.
pub fn interpret(event: &MidiEvent) -> Option<SurfaceAction> {
    match *event {
        MidiEvent::PitchBend { channel, value, .. } => {
            // Pitch bend arrives as -1..1; a fader is 0..1.
            let level = ((value + 1.0) * 0.5).clamp(0.0, 1.0);
            if (channel as usize) < STRIPS {
                Some(SurfaceAction::Fader {
                    strip: channel as usize,
                    value: level,
                })
            } else if channel as usize == STRIPS {
                Some(SurfaceAction::MasterFader { value: level })
            } else {
                None
            }
        }
        // A hand leaving a fader: the touch note's release.
        MidiEvent::NoteOff { note, .. }
            if (NOTE_TOUCH_FIRST..NOTE_TOUCH_FIRST + STRIPS as u8).contains(&note) =>
        {
            Some(SurfaceAction::Touch {
                strip: (note - NOTE_TOUCH_FIRST) as usize,
                on: false,
            })
        }
        MidiEvent::ControlChange { cc, value, .. }
            if (CC_VPOT_FIRST..CC_VPOT_FIRST + STRIPS as u8).contains(&cc) =>
        {
            // Relative: bit 6 is the direction (set is anticlockwise), the
            // rest how many clicks.
            let raw = (value.clamp(0.0, 1.0) * 127.0).round() as u8;
            let clicks = (raw & 0x3F) as i32;
            Some(SurfaceAction::VPot {
                strip: (cc - CC_VPOT_FIRST) as usize,
                clicks: if raw & 0x40 != 0 { -clicks } else { clicks },
            })
        }
        MidiEvent::NoteOn { note, velocity, .. } => {
            if (NOTE_TOUCH_FIRST..NOTE_TOUCH_FIRST + STRIPS as u8).contains(&note) {
                return Some(SurfaceAction::Touch {
                    strip: (note - NOTE_TOUCH_FIRST) as usize,
                    on: velocity > 0.0,
                });
            }
            if velocity <= 0.0 {
                return None;
            }
            match note {
                NOTE_REC_FIRST..=7 => Some(SurfaceAction::Arm {
                    strip: (note - NOTE_REC_FIRST) as usize,
                }),
                NOTE_SOLO_FIRST..=15 => Some(SurfaceAction::Solo {
                    strip: (note - NOTE_SOLO_FIRST) as usize,
                }),
                NOTE_MUTE_FIRST..=23 => Some(SurfaceAction::Mute {
                    strip: (note - NOTE_MUTE_FIRST) as usize,
                }),
                NOTE_VPOT_PUSH_FIRST..=39 => Some(SurfaceAction::VPotPush {
                    strip: (note - NOTE_VPOT_PUSH_FIRST) as usize,
                }),
                NOTE_BANK_LEFT => Some(SurfaceAction::BankLeft),
                NOTE_BANK_RIGHT => Some(SurfaceAction::BankRight),
                NOTE_REWIND => Some(SurfaceAction::Rewind),
                NOTE_FORWARD => Some(SurfaceAction::Forward),
                NOTE_STOP => Some(SurfaceAction::Stop),
                NOTE_PLAY => Some(SurfaceAction::Play),
                NOTE_RECORD => Some(SurfaceAction::Record),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Whether `event` is the desk's, so it plays no instrument and is not
/// learned: for HUI, also the halves (a zone, a fader's high bits) that
/// mean something only with what follows them.
pub fn claims(protocol: DeskProtocol, event: &MidiEvent) -> bool {
    match protocol {
        DeskProtocol::Mackie => interpret(event).is_some(),
        DeskProtocol::Hui => match *event {
            MidiEvent::NoteOn {
                channel: 0,
                note: 0,
                ..
            }
            | MidiEvent::NoteOff {
                channel: 0,
                note: 0,
                ..
            } => true,
            MidiEvent::ControlChange { channel: 0, cc, .. } => matches!(
                cc,
                HUI_CC_FADER_HIGH..=HUI_CC_FADER_HIGH_LAST
                    | HUI_CC_FADER_LOW..=HUI_CC_FADER_LOW_LAST
                    | HUI_CC_ZONE_IN
                    | HUI_CC_PORT_IN
                    | HUI_CC_VPOT_FIRST..=HUI_CC_VPOT_LAST
            ),
            _ => false,
        },
    }
}

/// Reads a desk in its protocol. A Mackie message stands on its own; a
/// HUI button is a zone and then a port, and a HUI fader its high bits and
/// then its low ones, so the reader keeps the first half until the second
/// comes.
#[derive(Default)]
pub struct DeskReader {
    hui_zone: u8,
    hui_fader_high: [u8; STRIPS + 1],
    /// When the HUI desk last answered a ping.
    hui_answered: Option<Instant>,
}

impl DeskReader {
    pub fn read(&mut self, protocol: DeskProtocol, event: &MidiEvent) -> Option<SurfaceAction> {
        match protocol {
            DeskProtocol::Mackie => interpret(event),
            DeskProtocol::Hui => self.read_hui(event),
        }
    }

    fn read_hui(&mut self, event: &MidiEvent) -> Option<SurfaceAction> {
        let (cc, value) = match *event {
            MidiEvent::NoteOn {
                channel: 0,
                note: 0,
                velocity,
                ..
            } if velocity > 0.0 => {
                let now = Instant::now();
                let back = self
                    .hui_answered
                    .is_none_or(|at| now.duration_since(at) > HUI_SILENCE);
                self.hui_answered = Some(now);
                return back.then_some(SurfaceAction::Hello);
            }
            MidiEvent::ControlChange {
                channel: 0,
                cc,
                value,
                ..
            } => (cc, value),
            _ => return None,
        };
        let raw = (value.clamp(0.0, 1.0) * 127.0).round() as u8;
        match cc {
            HUI_CC_FADER_HIGH..=HUI_CC_FADER_HIGH_LAST => {
                self.hui_fader_high[(cc - HUI_CC_FADER_HIGH) as usize] = raw;
                None
            }
            HUI_CC_FADER_LOW..=HUI_CC_FADER_LOW_LAST => {
                let strip = (cc - HUI_CC_FADER_LOW) as usize;
                let position = (u16::from(self.hui_fader_high[strip]) << 7) | u16::from(raw);
                let value = f32::from(position) / 16383.0;
                Some(if strip < STRIPS {
                    SurfaceAction::Fader { strip, value }
                } else {
                    SurfaceAction::MasterFader { value }
                })
            }
            HUI_CC_ZONE_IN => {
                self.hui_zone = raw;
                None
            }
            HUI_CC_PORT_IN => hui_button(self.hui_zone, raw & 0x07, raw & 0x40 != 0),
            HUI_CC_VPOT_FIRST..=HUI_CC_VPOT_LAST => {
                // Relative, the other way round from Mackie: bit 6 set is
                // clockwise, the rest how many clicks.
                let clicks = i32::from(raw & 0x3F);
                (clicks != 0).then(|| SurfaceAction::VPot {
                    strip: (cc - HUI_CC_VPOT_FIRST) as usize,
                    clicks: if raw & 0x40 != 0 { clicks } else { -clicks },
                })
            }
            _ => None,
        }
    }
}

/// A HUI button going down or up. As with Mackie, only the press acts,
/// except a fader's touch, whose release matters too.
fn hui_button(zone: u8, port: u8, down: bool) -> Option<SurfaceAction> {
    let strip = zone as usize;
    if strip < STRIPS && port == HUI_PORT_TOUCH {
        return Some(SurfaceAction::Touch { strip, on: down });
    }
    if !down {
        return None;
    }
    if strip < STRIPS {
        return match port {
            HUI_PORT_MUTE => Some(SurfaceAction::Mute { strip }),
            HUI_PORT_SOLO => Some(SurfaceAction::Solo { strip }),
            HUI_PORT_VPOT_PUSH => Some(SurfaceAction::VPotPush { strip }),
            HUI_PORT_ARM => Some(SurfaceAction::Arm { strip }),
            _ => None,
        };
    }
    match (zone, port) {
        (HUI_ZONE_BANK, 1) => Some(SurfaceAction::BankLeft),
        (HUI_ZONE_BANK, 3) => Some(SurfaceAction::BankRight),
        (HUI_ZONE_TRANSPORT, 1) => Some(SurfaceAction::Rewind),
        (HUI_ZONE_TRANSPORT, 2) => Some(SurfaceAction::Forward),
        (HUI_ZONE_TRANSPORT, 3) => Some(SurfaceAction::Stop),
        (HUI_ZONE_TRANSPORT, 4) => Some(SurfaceAction::Play),
        (HUI_ZONE_TRANSPORT, 5) => Some(SurfaceAction::Record),
        _ => None,
    }
}

/// A strip button with a light.
#[derive(Clone, Copy)]
enum StripButton {
    Arm,
    Solo,
    Mute,
}

impl DeskProtocol {
    /// A fader moved to `value` (0..1); strip eight is the master.
    fn fader(self, strip: usize, value: f32, out: &mut Vec<Vec<u8>>) {
        match self {
            DeskProtocol::Mackie => out.push(fader_feedback(strip, value).to_vec()),
            DeskProtocol::Hui => {
                let raw = (value.clamp(0.0, 1.0) * 16383.0).round() as u16;
                let strip = strip.min(STRIPS) as u8;
                out.push(vec![
                    0xB0,
                    HUI_CC_FADER_HIGH + strip,
                    ((raw >> 7) & 0x7F) as u8,
                ]);
                out.push(vec![0xB0, HUI_CC_FADER_LOW + strip, (raw & 0x7F) as u8]);
            }
        }
    }

    fn light(self, button: StripButton, strip: usize, lit: bool, out: &mut Vec<Vec<u8>>) {
        let strip = strip as u8 & 0x07;
        match self {
            DeskProtocol::Mackie => {
                let first = match button {
                    StripButton::Arm => NOTE_REC_FIRST,
                    StripButton::Solo => NOTE_SOLO_FIRST,
                    StripButton::Mute => NOTE_MUTE_FIRST,
                };
                out.push(button_feedback(first + strip, lit).to_vec());
            }
            DeskProtocol::Hui => {
                let port = match button {
                    StripButton::Arm => HUI_PORT_ARM,
                    StripButton::Solo => HUI_PORT_SOLO,
                    StripButton::Mute => HUI_PORT_MUTE,
                };
                out.push(vec![0xB0, HUI_CC_ZONE_OUT, strip]);
                out.push(vec![
                    0xB0,
                    HUI_CC_PORT_OUT,
                    if lit { 0x40 } else { 0 } | port,
                ]);
            }
        }
    }

    /// A V-pot ring; both protocols read the value the same way.
    fn ring(self, strip: usize, value: u8, out: &mut Vec<Vec<u8>>) {
        let first = match self {
            DeskProtocol::Mackie => CC_RING_FIRST,
            DeskProtocol::Hui => HUI_CC_RING_FIRST,
        };
        out.push(vec![0xB0, first + (strip as u8 & 0x07), value]);
    }

    /// Text on a scribble strip. A HUI strip has one row of four, so the
    /// second row (the level) is not shown there.
    fn text(self, strip: usize, row: usize, text: &str, out: &mut Vec<Vec<u8>>) {
        match self {
            DeskProtocol::Mackie => out.push(scribble(strip, row, text)),
            DeskProtocol::Hui if row == 0 => out.push(hui_scribble(strip, text)),
            DeskProtocol::Hui => {}
        }
    }
}

/// A fader position as the three bytes a desk expects back, so a
/// motorised fader moves when the mix changes rather than sitting where
/// the hand left it.
pub fn fader_feedback(strip: usize, value: f32) -> [u8; 3] {
    let level = value.clamp(0.0, 1.0);
    let raw = (level * 16383.0).round() as u16;
    [
        0xE0 | (strip as u8 & 0x0F),
        (raw & 0x7F) as u8,
        ((raw >> 7) & 0x7F) as u8,
    ]
}

/// A lit or dark button, as the desk expects it back.
pub fn button_feedback(note: u8, lit: bool) -> [u8; 3] {
    [0x90, note, if lit { 127 } else { 0 }]
}

/// Which tracks the eight strips are on, as ids.
///
/// Master is not one of them: it has the desk's own master fader, and
/// having it under a strip as well would mean two faders fighting.
pub fn bank_tracks(project: &Project, bank: usize) -> Vec<String> {
    project
        .tracks
        .iter()
        .filter(|t| t.kind.is_audio_bearing())
        .skip(bank * STRIPS)
        .take(STRIPS)
        .map(|t| t.id.clone())
        .collect()
}

/// How many banks of eight the song has, at least one.
pub fn bank_count(project: &Project) -> usize {
    let tracks = project
        .tracks
        .iter()
        .filter(|t| t.kind.is_audio_bearing())
        .count();
    tracks.div_ceil(STRIPS).max(1)
}

/// A fader position in decibels, the way the mixer stores it.
///
/// The curve matches the one MIDI Learn uses for a track fader, so the
/// desk and a learned knob move a fader the same distance.
pub fn fader_to_db(value: f32) -> f64 {
    // clamp keeps NaN as NaN; a broken value is the fader at the bottom.
    let value = if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    };
    -60.0 + value as f64 * 60.0
}

/// The reverse, for sending a fader back to the desk.
pub fn db_to_fader(db: f64) -> f32 {
    (((db + 60.0) / 60.0) as f32).clamp(0.0, 1.0)
}

/// One strip as the desk shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct StripView {
    pub fader: f32,
    pub volume_db: f64,
    /// The level as the scribble strip shows it, so a level that moves
    /// within the shown decimal sends nothing.
    pub level: String,
    pub pan: f64,
    pub muted: bool,
    pub soloed: bool,
    pub armed: bool,
    pub name: String,
}

/// What the desk should show for the bank it is on.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DeskFrame {
    pub strips: Vec<StripView>,
    pub master: f32,
}

/// The desk as the song is now, for `bank`.
pub fn desk_frame(project: &Project, bank: usize, master_db: f64) -> DeskFrame {
    let strips = bank_tracks(project, bank)
        .iter()
        .filter_map(|id| project.track(id))
        .map(|t| StripView {
            fader: db_to_fader(t.volume_db),
            volume_db: t.volume_db,
            level: level_text(t.volume_db),
            pan: t.pan,
            muted: t.muted,
            soloed: t.soloed,
            armed: t.armed,
            name: t.name.clone(),
        })
        .collect();
    DeskFrame {
        strips,
        master: db_to_fader(master_db),
    }
}

/// The messages that take the desk from what it showed (`old`, none for
/// a blank desk) to `new`: only what changed, and nothing for a fader with
/// a hand on it (`touched`, one bit per strip).
pub fn frame_messages(
    protocol: DeskProtocol,
    old: Option<&DeskFrame>,
    new: &DeskFrame,
    touched: u8,
) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for strip in 0..STRIPS {
        let now = new.strips.get(strip);
        let was = old.and_then(|o| o.strips.get(strip));
        if old.is_some() && now == was {
            continue;
        }
        let blank = StripView {
            fader: 0.0,
            volume_db: f64::NEG_INFINITY,
            level: String::new(),
            pan: 0.0,
            muted: false,
            soloed: false,
            armed: false,
            name: String::new(),
        };
        let v = now.unwrap_or(&blank);
        let changed = |f: &dyn Fn(&StripView) -> bool| was.is_none_or(|w| f(w) != f(v));
        // Compared as the desk sees it (14 bits), so automation moving a
        // fader by less than one step sends nothing.
        if touched & (1 << strip) == 0
            && was.is_none_or(|w| fader_feedback(strip, w.fader) != fader_feedback(strip, v.fader))
        {
            protocol.fader(strip, v.fader, &mut out);
        }
        if changed(&|s| s.armed) {
            protocol.light(StripButton::Arm, strip, v.armed, &mut out);
        }
        if changed(&|s| s.soloed) {
            protocol.light(StripButton::Solo, strip, v.soloed, &mut out);
        }
        if changed(&|s| s.muted) {
            protocol.light(StripButton::Mute, strip, v.muted, &mut out);
        }
        if was.is_none_or(|w| w.pan != v.pan) {
            protocol.ring(strip, pan_ring(v.pan, now.is_some()), &mut out);
        }
        if was.is_none_or(|w| w.name != v.name) {
            protocol.text(strip, 0, &v.name, &mut out);
        }
        if was.is_none_or(|w| w.level != v.level) {
            protocol.text(strip, 1, &v.level, &mut out);
        }
    }
    if old.is_none_or(|o| o.master != new.master) {
        protocol.fader(STRIPS, new.master, &mut out);
    }
    out
}

/// A level as the scribble strip shows it: one decimal, or -inf at the
/// bottom of the fader.
fn level_text(db: f64) -> String {
    if db <= -60.0 {
        "-inf".to_string()
    } else {
        format!("{db:.1}")
    }
}

/// Which open MIDI outputs are the desk: those named like an open input
/// (a desk shows up as an input and an output of the same name, give or
/// take "MIDIIN2 (X-Touch)" against "MIDIOUT2 (X-Touch)"), or the only
/// output open. Feedback used to go to every output, so a synth on the
/// clock port got pitch bends and notes thirty times a second.
pub fn desk_outputs(outputs: &[String], inputs: &[String]) -> Vec<String> {
    fn key(name: &str) -> String {
        name.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| {
                !w.is_empty()
                    && !matches!(*w, "in" | "out" | "input" | "output" | "port")
                    && !w.starts_with("midiin")
                    && !w.starts_with("midiout")
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
    let input_keys: Vec<String> = inputs.iter().map(|n| key(n)).collect();
    let matched: Vec<String> = outputs
        .iter()
        .filter(|o| input_keys.contains(&key(o)))
        .cloned()
        .collect();
    if !matched.is_empty() {
        return matched;
    }
    if outputs.len() == 1 {
        return outputs.to_vec();
    }
    Vec::new()
}

/// A V-pot ring showing a pan: one LED of eleven, the middle one centre,
/// and dark for a strip with no track under it.
fn pan_ring(pan: f64, lit: bool) -> u8 {
    if !lit {
        return 0;
    }
    let position = (6.0 + pan.clamp(-1.0, 1.0) * 5.0).round() as u8;
    position.clamp(1, 11)
}

/// Six characters of text on a strip's scribble strip, top row (0) or
/// bottom (1), as the Mackie display message. Each strip has seven
/// cells; the seventh stays blank so neighbours do not run together.
pub fn scribble(strip: usize, row: usize, text: &str) -> Vec<u8> {
    let offset = (row.min(1) * 56 + strip.min(STRIPS - 1) * 7) as u8;
    let mut msg = vec![0xF0, 0x00, 0x00, 0x66, 0x14, 0x12, offset];
    let cells: Vec<u8> = text
        .chars()
        .map(display_char)
        .chain(std::iter::repeat(b' '))
        .take(7)
        .enumerate()
        .map(|(i, c)| if i == 6 { b' ' } else { c })
        .collect();
    msg.extend(cells);
    msg.push(0xF7);
    msg
}

/// A strip's name on a HUI scribble strip: four characters.
pub fn hui_scribble(strip: usize, text: &str) -> Vec<u8> {
    let mut msg = vec![
        0xF0,
        0x00,
        0x00,
        0x66,
        0x05,
        0x00,
        0x10,
        strip.min(STRIPS - 1) as u8,
    ];
    msg.extend(
        text.chars()
            .map(display_char)
            .chain(std::iter::repeat(b' '))
            .take(4),
    );
    msg.push(0xF7);
    msg
}

/// A character as a desk's display shows it: plain ASCII, anything else
/// a question mark.
fn display_char(c: char) -> u8 {
    if c.is_ascii() && !c.is_ascii_control() {
        c as u8
    } else {
        b'?'
    }
}

/// Shared handle for the app and the dispatcher.
pub type SharedSurface = Arc<ControlSurface>;

#[cfg(test)]
mod tests {
    use super::*;

    fn bend(channel: u8, value: f32) -> MidiEvent {
        MidiEvent::PitchBend {
            timing: 0,
            channel,
            value,
        }
    }

    fn note_on(note: u8) -> MidiEvent {
        MidiEvent::NoteOn {
            timing: 0,
            channel: 0,
            note,
            velocity: 1.0,
        }
    }

    #[test]
    fn a_fader_is_pitch_bend_on_its_own_channel() {
        assert_eq!(
            interpret(&bend(2, 0.0)),
            Some(SurfaceAction::Fader {
                strip: 2,
                value: 0.5
            })
        );
        assert_eq!(
            interpret(&bend(0, -1.0)),
            Some(SurfaceAction::Fader {
                strip: 0,
                value: 0.0
            })
        );
    }

    #[test]
    fn the_ninth_fader_is_the_master() {
        assert_eq!(
            interpret(&bend(8, 1.0)),
            Some(SurfaceAction::MasterFader { value: 1.0 })
        );
    }

    #[test]
    fn the_buttons_are_notes_at_the_numbers_every_desk_sends() {
        assert_eq!(
            interpret(&note_on(0)),
            Some(SurfaceAction::Arm { strip: 0 })
        );
        assert_eq!(
            interpret(&note_on(11)),
            Some(SurfaceAction::Solo { strip: 3 })
        );
        assert_eq!(
            interpret(&note_on(23)),
            Some(SurfaceAction::Mute { strip: 7 })
        );
        assert_eq!(interpret(&note_on(94)), Some(SurfaceAction::Play));
        assert_eq!(interpret(&note_on(47)), Some(SurfaceAction::BankRight));
    }

    #[test]
    fn a_note_off_does_nothing() {
        let off = MidiEvent::NoteOff {
            timing: 0,
            channel: 0,
            note: 16,
            velocity: 0.0,
        };
        assert_eq!(interpret(&off), None);
        // A desk that sends note-on with no velocity means the same.
        let zero = MidiEvent::NoteOn {
            timing: 0,
            channel: 0,
            note: 16,
            velocity: 0.0,
        };
        assert_eq!(interpret(&zero), None);
    }

    #[test]
    fn a_fader_sent_back_is_the_same_position_it_came_in_at() {
        for value in [0.0f32, 0.25, 0.5, 0.75, 1.0] {
            let bytes = fader_feedback(3, value);
            assert_eq!(bytes[0], 0xE3);
            let raw = (bytes[1] as u16) | ((bytes[2] as u16) << 7);
            let back = raw as f32 / 16383.0;
            assert!((back - value).abs() < 0.001, "{value} came back as {back}");
        }
    }

    #[test]
    fn a_fader_and_its_level_agree_in_both_directions() {
        for value in [0.0f32, 0.3, 1.0] {
            let db = fader_to_db(value);
            assert!((db_to_fader(db) - value).abs() < 1e-6);
        }
    }

    fn strip(name: &str, db: f64) -> StripView {
        StripView {
            fader: db_to_fader(db),
            volume_db: db,
            level: level_text(db),
            pan: 0.0,
            muted: false,
            soloed: false,
            armed: false,
            name: name.into(),
        }
    }

    #[test]
    fn a_v_pot_is_relative_and_a_touch_is_a_note() {
        let cc = |cc: u8, raw: u8| MidiEvent::ControlChange {
            timing: 0,
            channel: 0,
            cc,
            value: raw as f32 / 127.0,
        };
        assert_eq!(
            interpret(&cc(17, 0x03)),
            Some(SurfaceAction::VPot {
                strip: 1,
                clicks: 3
            })
        );
        assert_eq!(
            interpret(&cc(17, 0x41)),
            Some(SurfaceAction::VPot {
                strip: 1,
                clicks: -1
            })
        );
        assert_eq!(
            interpret(&note_on(106)),
            Some(SurfaceAction::Touch { strip: 2, on: true })
        );
        assert_eq!(
            interpret(&MidiEvent::NoteOff {
                timing: 0,
                channel: 0,
                note: 106,
                velocity: 0.0
            }),
            Some(SurfaceAction::Touch {
                strip: 2,
                on: false
            })
        );
        assert_eq!(
            interpret(&note_on(33)),
            Some(SurfaceAction::VPotPush { strip: 1 })
        );
    }

    #[test]
    fn a_blank_desk_gets_everything_and_after_that_only_what_changed() {
        let first = DeskFrame {
            strips: vec![strip("Kick", -6.0), strip("Bass", 0.0)],
            master: 1.0,
        };
        let all = frame_messages(DeskProtocol::Mackie, None, &first, 0);
        assert!(all.contains(&scribble(0, 0, "Kick")), "names on the strips");
        assert!(
            all.contains(&fader_feedback(1, 1.0).to_vec()),
            "faders moved"
        );
        assert!(
            all.contains(&scribble(2, 0, "")),
            "an empty strip is cleared"
        );

        let mut next = first.clone();
        next.strips[1].muted = true;
        let changes = frame_messages(DeskProtocol::Mackie, Some(&first), &next, 0);
        assert_eq!(
            changes,
            vec![button_feedback(NOTE_MUTE_FIRST + 1, true).to_vec()]
        );
    }

    #[test]
    fn a_fader_with_a_hand_on_it_is_not_moved() {
        let first = DeskFrame {
            strips: vec![strip("Kick", -6.0)],
            master: 1.0,
        };
        let mut next = first.clone();
        next.strips[0] = strip("Kick", -3.0);
        let changes = frame_messages(DeskProtocol::Mackie, Some(&first), &next, 0b0000_0001);
        assert!(
            !changes.iter().any(|m| m[0] == 0xE0),
            "no fader message while touched"
        );
        assert!(
            changes.contains(&scribble(0, 1, "-3.0")),
            "the level still shows"
        );
    }

    #[test]
    fn the_scribble_strip_is_six_characters_and_a_gap() {
        let m = scribble(1, 1, "Lead synth");
        assert_eq!(&m[..7], &[0xF0, 0x00, 0x00, 0x66, 0x14, 0x12, 63]);
        assert_eq!(&m[7..14], b"Lead s ");
        assert_eq!(*m.last().unwrap(), 0xF7);
        assert_eq!(&scribble(0, 0, "Bassé")[7..14], b"Bass?  ");
    }

    #[test]
    fn feedback_goes_to_the_desk_not_every_output() {
        let outs = vec!["MIDIOUT2 (X-Touch)".to_string(), "Synth".to_string()];
        let ins = vec!["MIDIIN2 (X-Touch)".to_string()];
        assert_eq!(
            desk_outputs(&outs, &ins),
            vec!["MIDIOUT2 (X-Touch)".to_string()]
        );
        assert_eq!(
            desk_outputs(&["Only one".to_string()], &[]),
            vec!["Only one".to_string()]
        );
        assert!(
            desk_outputs(&outs, &["Keystation".to_string()]).is_empty(),
            "no guessing"
        );
    }

    #[test]
    fn a_level_that_moves_within_its_decimal_sends_nothing() {
        let first = DeskFrame {
            strips: vec![strip("Kick", -6.01)],
            master: 1.0,
        };
        let next = DeskFrame {
            strips: vec![strip("Kick", -6.02)],
            master: 1.0,
        };
        // The fader itself (touched here) is a real step on a 14-bit desk;
        // the level text reads -6.0 both times and is not sent again.
        assert!(frame_messages(DeskProtocol::Mackie, Some(&first), &next, 0b1).is_empty());
    }

    fn hui_cc(cc: u8, raw: u8) -> MidiEvent {
        MidiEvent::ControlChange {
            timing: 0,
            channel: 0,
            cc,
            value: raw as f32 / 127.0,
        }
    }

    /// A HUI button: its zone, then its port with bit 6 for down.
    fn hui_press(reader: &mut DeskReader, zone: u8, port: u8, down: bool) -> Option<SurfaceAction> {
        let first = reader.read(DeskProtocol::Hui, &hui_cc(0x0F, zone));
        assert_eq!(first, None, "a zone alone does nothing");
        reader.read(
            DeskProtocol::Hui,
            &hui_cc(0x2F, if down { 0x40 } else { 0 } | port),
        )
    }

    #[test]
    fn a_hui_button_is_a_zone_then_a_port() {
        let mut r = DeskReader::default();
        assert_eq!(
            hui_press(&mut r, 2, 2, true),
            Some(SurfaceAction::Mute { strip: 2 })
        );
        assert_eq!(
            hui_press(&mut r, 2, 2, false),
            None,
            "the release does nothing"
        );
        assert_eq!(
            hui_press(&mut r, 5, 3, true),
            Some(SurfaceAction::Solo { strip: 5 })
        );
        assert_eq!(
            hui_press(&mut r, 0, 7, true),
            Some(SurfaceAction::Arm { strip: 0 })
        );
        assert_eq!(
            hui_press(&mut r, 7, 5, true),
            Some(SurfaceAction::VPotPush { strip: 7 })
        );
        assert_eq!(hui_press(&mut r, 0x0E, 4, true), Some(SurfaceAction::Play));
        assert_eq!(hui_press(&mut r, 0x0E, 3, true), Some(SurfaceAction::Stop));
        assert_eq!(
            hui_press(&mut r, 0x0E, 5, true),
            Some(SurfaceAction::Record)
        );
        assert_eq!(
            hui_press(&mut r, 0x0E, 1, true),
            Some(SurfaceAction::Rewind)
        );
        assert_eq!(
            hui_press(&mut r, 0x0A, 1, true),
            Some(SurfaceAction::BankLeft)
        );
        assert_eq!(
            hui_press(&mut r, 0x0A, 3, true),
            Some(SurfaceAction::BankRight)
        );
    }

    #[test]
    fn a_hui_touch_counts_going_down_and_coming_up() {
        let mut r = DeskReader::default();
        assert_eq!(
            hui_press(&mut r, 4, 0, true),
            Some(SurfaceAction::Touch { strip: 4, on: true })
        );
        assert_eq!(
            hui_press(&mut r, 4, 0, false),
            Some(SurfaceAction::Touch {
                strip: 4,
                on: false
            })
        );
    }

    #[test]
    fn a_hui_fader_is_its_high_bits_then_its_low() {
        let mut r = DeskReader::default();
        assert_eq!(r.read(DeskProtocol::Hui, &hui_cc(0x03, 0x40)), None);
        assert_eq!(
            r.read(DeskProtocol::Hui, &hui_cc(0x23, 0x00)),
            Some(SurfaceAction::Fader {
                strip: 3,
                value: 8192.0 / 16383.0
            })
        );
        r.read(DeskProtocol::Hui, &hui_cc(0x00, 0x7F));
        assert_eq!(
            r.read(DeskProtocol::Hui, &hui_cc(0x20, 0x7F)),
            Some(SurfaceAction::Fader {
                strip: 0,
                value: 1.0
            })
        );
    }

    #[test]
    fn a_hui_fader_sent_back_is_the_same_position_it_came_in_at() {
        for value in [0.0f32, 0.25, 0.5, 0.75, 1.0] {
            let mut out = Vec::new();
            DeskProtocol::Hui.fader(6, value, &mut out);
            assert_eq!(out.len(), 2);
            assert_eq!(&out[0][..2], &[0xB0, 0x06]);
            assert_eq!(&out[1][..2], &[0xB0, 0x26]);
            let mut r = DeskReader::default();
            r.read(DeskProtocol::Hui, &hui_cc(out[0][1], out[0][2]));
            let Some(SurfaceAction::Fader {
                strip: 6,
                value: back,
            }) = r.read(DeskProtocol::Hui, &hui_cc(out[1][1], out[1][2]))
            else {
                panic!("not read back as fader 6");
            };
            assert!((back - value).abs() < 0.001, "{value} came back as {back}");
        }
    }

    #[test]
    fn a_hui_v_pot_turns_clockwise_with_bit_six_set() {
        let mut r = DeskReader::default();
        assert_eq!(
            r.read(DeskProtocol::Hui, &hui_cc(0x41, 0x42)),
            Some(SurfaceAction::VPot {
                strip: 1,
                clicks: 2
            })
        );
        assert_eq!(
            r.read(DeskProtocol::Hui, &hui_cc(0x41, 0x01)),
            Some(SurfaceAction::VPot {
                strip: 1,
                clicks: -1
            })
        );
    }

    #[test]
    fn a_hui_desk_answering_the_ping_after_a_silence_is_sent_everything() {
        let mut r = DeskReader::default();
        let answer = MidiEvent::NoteOn {
            timing: 0,
            channel: 0,
            note: 0,
            velocity: 1.0,
        };
        assert_eq!(
            r.read(DeskProtocol::Hui, &answer),
            Some(SurfaceAction::Hello)
        );
        assert_eq!(
            r.read(DeskProtocol::Hui, &answer),
            None,
            "a second later: still there"
        );
        assert!(
            claims(DeskProtocol::Hui, &answer),
            "the answer plays no note"
        );
        // To a Mackie desk the same note is strip one's arm button.
        assert_eq!(interpret(&answer), Some(SurfaceAction::Arm { strip: 0 }));
    }

    #[test]
    fn hui_claims_its_controllers_on_the_first_channel_only() {
        assert!(claims(DeskProtocol::Hui, &hui_cc(0x0F, 3)));
        assert!(claims(DeskProtocol::Hui, &hui_cc(0x25, 3)));
        let other_channel = MidiEvent::ControlChange {
            timing: 0,
            channel: 1,
            cc: 0x0F,
            value: 0.0,
        };
        assert!(!claims(DeskProtocol::Hui, &other_channel));
        // Pitch bend is a keyboard's, not a HUI desk's.
        assert!(!claims(DeskProtocol::Hui, &bend(0, 0.0)));
        assert!(claims(DeskProtocol::Mackie, &bend(0, 0.0)));
    }

    #[test]
    fn a_hui_desk_gets_lights_as_zone_and_port_and_four_letter_names() {
        let first = DeskFrame {
            strips: vec![strip("Kick", -6.0), strip("Bassline", 0.0)],
            master: 1.0,
        };
        let all = frame_messages(DeskProtocol::Hui, None, &first, 0);
        assert!(all.contains(&hui_scribble(0, "Kick")));
        assert!(all.contains(&vec![
            0xF0, 0x00, 0x00, 0x66, 0x05, 0x00, 0x10, 1, b'B', b'a', b's', b's', 0xF7
        ]));
        assert!(
            !all.iter().any(|m| m.len() > 5 && m[4] == 0x14),
            "no Mackie display messages"
        );
        let mut next = first.clone();
        next.strips[1].soloed = true;
        assert_eq!(
            frame_messages(DeskProtocol::Hui, Some(&first), &next, 0),
            vec![vec![0xB0, 0x0C, 1], vec![0xB0, 0x2C, 0x43]]
        );
        let mut held = first.clone();
        held.strips[0] = strip("Kick", -3.0);
        let changes = frame_messages(DeskProtocol::Hui, Some(&first), &held, 0b1);
        assert!(
            !changes
                .iter()
                .any(|m| m[0] == 0xB0 && (m[1] == 0x00 || m[1] == 0x20)),
            "no fader message while touched"
        );
        assert!(changes.is_empty(), "the level row is not on a HUI desk");
    }
}
