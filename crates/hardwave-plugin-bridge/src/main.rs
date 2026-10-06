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

use std::io::{stdin, stdout, BufReader, BufWriter};

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
    let descriptor = descriptor(path, id);
    let mut plugin = match load(&descriptor) {
        Ok(plugin) => plugin,
        Err(e) => {
            let mut out = BufWriter::new(stdout());
            let _ = Frame::new(RES_ERROR, e.as_bytes().to_vec()).write(&mut out);
            return 3;
        }
    };

    let mut reader = BufReader::new(stdin());
    let mut writer = BufWriter::new(stdout());
    let mut left = vec![0.0f32; 4096];
    let mut right = vec![0.0f32; 4096];
    let mut outputs = vec![Vec::new(), Vec::new()];

    // The host going away, or the stream ending, ends this process
    // too: there is nothing here without it.
    while let Ok(Some(frame)) = Frame::read(&mut reader) {
        match frame.kind {
            REQ_SHUTDOWN => break,
            REQ_ACTIVATE => {
                if let Some((sample_rate, max_block)) = frame.read_activate() {
                    let block = if max_block == 0 { 4096 } else { max_block };
                    if left.len() < block as usize {
                        left.resize(block as usize, 0.0);
                        right.resize(block as usize, 0.0);
                    }
                    let _ = plugin.activate(sample_rate, block);
                }
                let _ = Frame::new(RES_OK, Vec::new()).write(&mut writer);
            }
            REQ_SET_PARAM => {
                if let Some((id, value)) = frame.read_set_param() {
                    plugin.set_parameter_value(id, value);
                }
            }
            REQ_SET_STATE => {
                let _ = plugin.set_state(&frame.payload);
            }
            REQ_GET_STATE => {
                let _ = Frame::new(RES_STATE, plugin.get_state()).write(&mut writer);
            }
            REQ_AUDIO => {
                let frames = frame.payload.len() / 8;
                if left.len() < frames {
                    left.resize(frames, 0.0);
                    right.resize(frames, 0.0);
                }
                frame.read_audio(&mut left[..frames], &mut right[..frames]);
                let inputs: [&[f32]; 2] = [&left[..frames], &right[..frames]];
                plugin.process(&inputs, &mut outputs, &[], &mut Vec::new(), frames);
                let out_l = outputs.first().map(|c| c.as_slice()).unwrap_or(&[]);
                let out_r = outputs.get(1).map(|c| c.as_slice()).unwrap_or(out_l);
                let mut reply = Frame::audio(out_l, out_r);
                reply.kind = RES_AUDIO;
                if reply.write(&mut writer).is_err() {
                    break;
                }
            }
            _ => {}
        }
    }
    0
}
