//! The helper process that hosts one plug-in.
//!
//! Built for 32-bit on Windows, where it is the only way to run a
//! plug-in that was never rebuilt: a 64-bit process cannot load a
//! 32-bit binary, however much the user would like it to. Built for
//! the host's own architecture everywhere else, where it is a
//! sandbox: a crash in here takes this process and leaves the song
//! playing.
//!
//! It speaks the frames in `bridge_protocol`: audio in, audio out,
//! parameter moves, state. Nothing else lives in this process.

use std::io::{stdin, BufWriter};

use hardwave_plugin_host::bridge_protocol::*;
use hardwave_plugin_host::types::{HostedPlugin, PluginCategory, PluginDescriptor, PluginFormat};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1) else {
        eprintln!("usage: hardwave-plugin-bridge <plug-in path> [id]");
        std::process::exit(2);
    };
    let id = args.get(2).cloned().unwrap_or_else(|| path.clone());
    std::process::exit(run(path, &id));
}

fn descriptor(path: &str, id: &str) -> PluginDescriptor {
    let path_buf = std::path::PathBuf::from(path);
    let format = if path.to_lowercase().ends_with(".clap") {
        PluginFormat::Clap
    } else {
        PluginFormat::Vst3
    };
    PluginDescriptor {
        id: id.to_string(),
        name: path_buf
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(id)
            .to_string(),
        vendor: String::new(),
        version: String::new(),
        format,
        path: path_buf,
        category: PluginCategory::Effect,
        num_inputs: 2,
        num_outputs: 2,
        has_midi_input: false,
        has_editor: false,
    }
}

fn load(descriptor: &PluginDescriptor) -> Result<Box<dyn HostedPlugin>, String> {
    match descriptor.format {
        PluginFormat::Clap => {
            hardwave_plugin_host::clap_instance::ClapPluginInstance::load(descriptor.clone())
                .map(|p| Box::new(p) as Box<dyn HostedPlugin>)
        }
        _ => hardwave_plugin_host::vst3::Vst3PluginInstance::load(descriptor.clone())
            .map(|p| Box::new(p) as Box<dyn HostedPlugin>),
    }
}

fn run(path: &str, id: &str) -> i32 {
    // Frames get a private copy of stdout before any plug-in code runs;
    // what the plug-in prints goes nowhere.
    let frames_out = match hardwave_plugin_host::bridge_child::private_stdout() {
        Ok(out) => out,
        Err(e) => {
            eprintln!("hardwave-plugin-bridge: no stream for frames: {e}");
            return 4;
        }
    };
    let descriptor = descriptor(path, id);
    let mut plugin = match load(&descriptor) {
        Ok(plugin) => plugin,
        Err(e) => {
            let mut out = BufWriter::new(frames_out);
            let _ = Frame::new(RES_ERROR, e.as_bytes().to_vec()).write(&mut out);
            return 3;
        }
    };
    hardwave_plugin_host::bridge_child::serve(plugin.as_mut(), stdin(), frames_out)
}
