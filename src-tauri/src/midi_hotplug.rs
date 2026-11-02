//! Noticing a MIDI input that appears while the DAW runs, and saying what
//! kind it is.
//!
//! A controller plugged in after setup showed up in Settings > MIDI with a
//! Connect button and stayed off, so its keys played nothing until the
//! producer found that page. The hot-plug pass that reopens wanted inputs
//! every two seconds now also says which inputs are new, as a
//! `midi-input-appeared` event; the window decides what to do with each
//! (open it with a message, offer a desk as a control surface, or leave a
//! virtual port alone), against the choices it remembers per device.

use serde::Serialize;

/// What an input is, by its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InputKind {
    /// A keyboard, pad or anything else that plays.
    Keyboard,
    /// A control surface (an X-Touch, a Mackie-style desk): its buttons
    /// send notes, so it is offered as a surface, not opened as a keyboard.
    Desk,
    /// A virtual port (loopMIDI and the like), which could feed the DAW's
    /// own MIDI back into it.
    Virtual,
}

/// One MIDI input and its kind, for the window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InputSeen {
    pub port: String,
    pub kind: InputKind,
}

/// The names virtual ports go by, as words written without their spaces.
/// "loopMIDI Port", "LoopBe Internal MIDI", "Midi Through Port-0", "IAC
/// Driver Bus 1", "VirMIDI 1-0" and "Bome MIDI Translator 1 Virtual In"
/// are virtual; "Akai MPK Mini mk3" is not.
const VIRTUAL_NAMES: [&str; 7] = [
    "loopmidi",
    "loopbe",
    "midithrough",
    "iacdriver",
    "virmidi",
    "virtual",
    "rtpmidi",
];

/// Whether an input is named like a virtual port.
pub fn looks_virtual(port: &str) -> bool {
    let joined: String = port
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    VIRTUAL_NAMES.iter().any(|name| joined.contains(name))
}

/// What an input is, by its name. A desk is known by the control surface's
/// own list of desk names, the one the desk input picker uses.
pub fn kind_of(port: &str) -> InputKind {
    if looks_virtual(port) {
        InputKind::Virtual
    } else if crate::control_surface::looks_like_desk(port) {
        InputKind::Desk
    } else {
        InputKind::Keyboard
    }
}

/// Every input in a list, with its kind.
pub fn kinds(ports: &[String]) -> Vec<InputSeen> {
    ports
        .iter()
        .map(|port| InputSeen {
            port: port.clone(),
            kind: kind_of(port),
        })
        .collect()
}

/// The inputs seen at the last look, to tell which ones are new.
#[derive(Default)]
pub struct PortWatch {
    known: Vec<String>,
}

impl PortWatch {
    pub fn new() -> Self {
        Self::default()
    }

    /// The inputs in `available` that were not there at the last look. The
    /// first look counts every input as new: the window treats an input it
    /// has dealt with before as known, so that costs nothing, and a
    /// controller plugged in while the window was still loading is not
    /// missed. One unplugged and plugged in again is new again.
    pub fn arrivals(&mut self, available: &[String]) -> Vec<String> {
        let new: Vec<String> = available
            .iter()
            .filter(|port| !self.known.contains(port))
            .cloned()
            .collect();
        self.known = available.to_vec();
        new
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ports(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn a_controller_plugged_in_later_is_new_once() {
        let mut watch = PortWatch::new();
        assert_eq!(
            watch.arrivals(&ports(&["Novation Launchkey 49"])),
            ports(&["Novation Launchkey 49"]),
            "what is there at the first look"
        );
        assert!(watch
            .arrivals(&ports(&["Novation Launchkey 49"]))
            .is_empty());
        assert_eq!(
            watch.arrivals(&ports(&["Novation Launchkey 49", "Akai MPK Mini mk3"])),
            ports(&["Akai MPK Mini mk3"]),
            "the one plugged in"
        );
        assert!(
            watch
                .arrivals(&ports(&["Akai MPK Mini mk3", "Novation Launchkey 49"]))
                .is_empty(),
            "the order the system lists them in does not matter"
        );
    }

    #[test]
    fn unplugged_and_plugged_in_again_is_new_again() {
        let mut watch = PortWatch::new();
        watch.arrivals(&ports(&["Akai MPK Mini mk3"]));
        assert!(watch.arrivals(&[]).is_empty(), "unplugged: nothing new");
        assert_eq!(
            watch.arrivals(&ports(&["Akai MPK Mini mk3"])),
            ports(&["Akai MPK Mini mk3"])
        );
    }

    #[test]
    fn a_desk_and_a_virtual_port_are_told_apart_from_a_keyboard() {
        for keyboard in [
            "Akai MPK Mini mk3",
            "Novation Launchkey 49",
            "MIDIIN2 (Launchkey MK3 49)",
            "Arturia KeyLab Essential 61",
        ] {
            assert_eq!(kind_of(keyboard), InputKind::Keyboard, "{keyboard}");
        }
        for desk in [
            "X-Touch",
            "MCU Pro",
            "iCON Platform M+ V2.15",
            "PreSonus FaderPort 8",
            "SSL UF8",
        ] {
            assert_eq!(kind_of(desk), InputKind::Desk, "{desk}");
        }
        for port in [
            "loopMIDI Port",
            "LoopBe Internal MIDI",
            "Midi Through Port-0",
            "IAC Driver Bus 1",
            "VirMIDI 1-0",
            "Bome MIDI Translator 1 Virtual In",
            "rtpMIDI session 1",
        ] {
            assert_eq!(kind_of(port), InputKind::Virtual, "{port}");
        }
    }

    #[test]
    fn the_window_is_sent_lowercase_kinds() {
        let seen = kinds(&ports(&["X-Touch"]));
        assert_eq!(
            serde_json::to_string(&seen).unwrap(),
            r#"[{"port":"X-Touch","kind":"desk"}]"#
        );
    }
}
