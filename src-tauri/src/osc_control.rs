//! Driving the DAW from TouchOSC and anything else that speaks OSC.
//!
//! A phone or tablet on the same network becomes a remote: transport,
//! faders, mutes, the tempo. The packet format already existed in
//! `hardwave_midi::osc`; what was missing was something listening, and
//! a decision about which address means what.
//!
//! The addresses follow what a desk already does, so an OSC layout and
//! a Mackie desk end up in the same dispatcher rather than two:
//!
//! ```text
//! /hardwave/play                     1.0 or no argument
//! /hardwave/stop
//! /hardwave/record
//! /hardwave/tempo                    bpm, as a float
//! /hardwave/goto                     beats from the start
//! /hardwave/bank/left  /bank/right
//! /hardwave/master/volume            0..1
//! /hardwave/track/3/volume           0..1, strips count from 1
//! /hardwave/track/3/pan              -1..1
//! /hardwave/track/3/mute             press, 1.0
//! /hardwave/track/3/solo
//! /hardwave/track/3/arm
//! ```

use crate::control_surface::SurfaceAction;
use hardwave_midi::osc::{OscArg, OscMessage};

/// The port TouchOSC sends to by default.
pub const DEFAULT_PORT: u16 = 9000;

/// The first argument as a number, whatever type it arrived as. A
/// button with no argument counts as pressed, because some layouts
/// send a bare address.
fn value_of(message: &OscMessage) -> f32 {
    match message.args.first() {
        Some(OscArg::Float32(v)) => *v,
        Some(OscArg::Int32(v)) => *v as f32,
        Some(OscArg::String(_)) | None => 1.0,
    }
}

/// Read one OSC message as a desk message, or nothing when the address
/// is not one we know.
///
/// Unknown addresses are ignored rather than guessed at: a layout sends
/// whatever its designer typed, and acting on a near miss would move
/// the wrong fader.
pub fn interpret(message: &OscMessage) -> Option<SurfaceAction> {
    let address = message.address.trim_end_matches('/');
    let rest = address.strip_prefix("/hardwave/")?;
    let value = value_of(message);
    // A value that is not a number is not a fader position; NaN passes
    // straight through clamp and would reach the mix.
    if !value.is_finite() {
        return None;
    }
    // A button sends 1.0 on press and 0.0 on release. Acting on both
    // would toggle twice and end up where it started.
    let pressed = value >= 0.5;

    let parts: Vec<&str> = rest.split('/').collect();
    match parts.as_slice() {
        ["play"] if pressed => Some(SurfaceAction::Play),
        ["stop"] if pressed => Some(SurfaceAction::Stop),
        ["record"] if pressed => Some(SurfaceAction::Record),
        ["rewind"] if pressed => Some(SurfaceAction::Rewind),
        ["forward"] if pressed => Some(SurfaceAction::Forward),
        ["bank", "left"] if pressed => Some(SurfaceAction::BankLeft),
        ["bank", "right"] if pressed => Some(SurfaceAction::BankRight),
        ["tempo"] => {
            let bpm = value as f64;
            // A tempo outside what a DAW can play is a layout sending
            // its own range, not a tempo.
            if (20.0..=999.0).contains(&bpm) {
                Some(SurfaceAction::Tempo { bpm })
            } else {
                None
            }
        }
        ["goto"] => Some(SurfaceAction::Goto {
            beats: (value as f64).max(0.0),
        }),
        ["master", "volume"] => Some(SurfaceAction::MasterFader {
            value: value.clamp(0.0, 1.0),
        }),
        ["track", number, what] => {
            // Strips count from 1 in a layout, from 0 in the code.
            let strip = number.parse::<usize>().ok()?.checked_sub(1)?;
            match *what {
                "volume" => Some(SurfaceAction::Fader {
                    strip,
                    value: value.clamp(0.0, 1.0),
                }),
                "pan" => Some(SurfaceAction::Pan {
                    strip,
                    value: value.clamp(-1.0, 1.0),
                }),
                "mute" if pressed => Some(SurfaceAction::Mute { strip }),
                "solo" if pressed => Some(SurfaceAction::Solo { strip }),
                "arm" if pressed => Some(SurfaceAction::Arm { strip }),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Listen for OSC on a UDP port and apply what arrives.
///
/// Its own thread with a short read timeout, so switching it off takes
/// effect without waiting for a packet that may never come. A packet
/// that does not parse is dropped: anything can send to a UDP port, and
/// a stray one should not reach the mixer.
/// Whether a sender is on this machine or the local network: loopback,
/// a private range, or link-local. The internet never drives the DAW.
pub fn from_the_local_network(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback()
                // Unique local (fc00::/7) and link-local (fe80::/10).
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_private() || v4.is_loopback())
        }
    }
}

pub fn spawn_listener(
    engine: std::sync::Arc<parking_lot::Mutex<hardwave_engine::DawEngine>>,
    surface: crate::control_surface::SharedSurface,
    midi_out: std::sync::Arc<parking_lot::Mutex<hardwave_midi::output::MidiOutputManager>>,
    enabled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    port: u16,
) -> Result<(), String> {
    use std::net::UdpSocket;
    use std::sync::atomic::Ordering;

    let socket = UdpSocket::bind(("0.0.0.0", port))
        .map_err(|e| format!("could not listen for OSC on port {port}: {e}"))?;
    socket
        .set_read_timeout(Some(std::time::Duration::from_millis(200)))
        .map_err(|e| format!("could not set the OSC read timeout: {e}"))?;

    std::thread::Builder::new()
        .name("hardwave-osc".into())
        .spawn(move || {
            let mut buffer = [0u8; 4096];
            // The controller this session answers to; switching OSC off
            // and on again lets another one pair.
            let mut paired: Option<std::net::IpAddr> = None;
            while enabled.load(Ordering::Relaxed) {
                let (read, from) = match socket.recv_from(&mut buffer) {
                    Ok(received) => received,
                    // A timeout is the normal case: nothing was sent.
                    Err(_) => continue,
                };
                // Only the local network, and only the first device that
                // spoke: anyone else on the same Wi-Fi, or anything from
                // the internet, is not a desk this person set up.
                if !from_the_local_network(from.ip()) {
                    continue;
                }
                match paired {
                    None => {
                        paired = Some(from.ip());
                        log::info!("OSC: paired with the first controller that spoke");
                    }
                    Some(ip) if ip != from.ip() => continue,
                    Some(_) => {}
                }
                let Ok(message) = OscMessage::from_bytes(&buffer[..read]) else {
                    continue;
                };
                if let Some(action) = interpret(&message) {
                    crate::midi_map::apply_surface_action(&engine, &surface, &midi_out, action);
                }
            }
        })
        .map_err(|e| format!("could not start the OSC thread: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn float(address: &str, value: f32) -> OscMessage {
        OscMessage::new(address).push_float(value)
    }

    #[test]
    fn a_value_that_is_not_a_number_moves_nothing() {
        assert_eq!(interpret(&float("/hardwave/master/volume", f32::NAN)), None);
        assert_eq!(
            interpret(&float("/hardwave/track/1/volume", f32::INFINITY)),
            None
        );
    }

    #[test]
    fn only_the_local_network_may_drive_the_daw() {
        use std::net::IpAddr;
        for ok in [
            "127.0.0.1",
            "192.168.1.20",
            "10.0.0.5",
            "172.16.3.4",
            "169.254.1.1",
            "::1",
            "fe80::1",
            "fd00::5",
        ] {
            assert!(
                from_the_local_network(ok.parse::<IpAddr>().unwrap()),
                "{ok}"
            );
        }
        for no in ["8.8.8.8", "178.104.2.34", "2a01:4f8::1", "100.64.0.1"] {
            assert!(
                !from_the_local_network(no.parse::<IpAddr>().unwrap()),
                "{no}"
            );
        }
    }

    #[test]
    fn transport_buttons_are_read() {
        assert_eq!(
            interpret(&float("/hardwave/play", 1.0)),
            Some(SurfaceAction::Play)
        );
        assert_eq!(
            interpret(&float("/hardwave/stop", 1.0)),
            Some(SurfaceAction::Stop)
        );
        assert_eq!(
            interpret(&OscMessage::new("/hardwave/record")),
            Some(SurfaceAction::Record),
            "a bare address counts as a press"
        );
    }

    #[test]
    fn a_button_being_let_go_does_nothing() {
        assert_eq!(interpret(&float("/hardwave/play", 0.0)), None);
        assert_eq!(interpret(&float("/hardwave/track/2/mute", 0.0)), None);
    }

    #[test]
    fn a_fader_is_read_per_strip_counting_from_one() {
        assert_eq!(
            interpret(&float("/hardwave/track/1/volume", 0.75)),
            Some(SurfaceAction::Fader {
                strip: 0,
                value: 0.75
            })
        );
        assert_eq!(
            interpret(&float("/hardwave/track/8/volume", 0.0)),
            Some(SurfaceAction::Fader {
                strip: 7,
                value: 0.0
            })
        );
    }

    #[test]
    fn strip_zero_is_not_a_strip() {
        assert_eq!(interpret(&float("/hardwave/track/0/volume", 0.5)), None);
    }

    #[test]
    fn a_fader_out_of_range_is_brought_back_in() {
        assert_eq!(
            interpret(&float("/hardwave/track/2/volume", 1.8)),
            Some(SurfaceAction::Fader {
                strip: 1,
                value: 1.0
            })
        );
        assert_eq!(
            interpret(&float("/hardwave/track/2/pan", -4.0)),
            Some(SurfaceAction::Pan {
                strip: 1,
                value: -1.0
            })
        );
    }

    #[test]
    fn the_tempo_is_taken_only_when_it_is_a_tempo() {
        assert_eq!(
            interpret(&float("/hardwave/tempo", 174.0)),
            Some(SurfaceAction::Tempo { bpm: 174.0 })
        );
        assert_eq!(
            interpret(&float("/hardwave/tempo", 0.5)),
            None,
            "a layout sending 0..1 is not sending a tempo"
        );
    }

    #[test]
    fn an_integer_argument_works_as_well_as_a_float() {
        assert_eq!(
            interpret(&OscMessage::new("/hardwave/tempo").push_int(150)),
            Some(SurfaceAction::Tempo { bpm: 150.0 })
        );
    }

    #[test]
    fn an_address_we_do_not_know_is_left_alone() {
        assert_eq!(interpret(&float("/hardwave/mystery", 1.0)), None);
        assert_eq!(interpret(&float("/other/app/play", 1.0)), None);
        assert_eq!(interpret(&float("/hardwave/track/two/volume", 1.0)), None);
    }

    #[test]
    fn a_trailing_slash_is_tolerated() {
        assert_eq!(
            interpret(&float("/hardwave/play/", 1.0)),
            Some(SurfaceAction::Play)
        );
    }

    #[test]
    fn the_playhead_can_be_sent_somewhere_but_never_before_the_start() {
        assert_eq!(
            interpret(&float("/hardwave/goto", 32.0)),
            Some(SurfaceAction::Goto { beats: 32.0 })
        );
        assert_eq!(
            interpret(&float("/hardwave/goto", -5.0)),
            Some(SurfaceAction::Goto { beats: 0.0 })
        );
    }
}
