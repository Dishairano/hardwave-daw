// Phase 6 — MANUAL tests for every UI / audio / visual roadmap item that
// can only be verified by human eyes, ears, or mouse/keyboard interaction.
// Each test has concrete step-by-step instructions; the tester clicks Pass
// or Fail.

import type { TestDef } from './tests'

export const PHASE6_TESTS: TestDef[] = []

// ─────────────────────────────────────────────────────────────────────────────
// 6A — Transport / toolbar / keyboard shortcuts
// ─────────────────────────────────────────────────────────────────────────────

PHASE6_TESTS.push(
  {
    id: 'p6_space_plays_stops',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Space bar play/stop',
    title: 'Space bar toggles play/stop',
    instructions: 'Focus the arrangement, press Space. The play button should turn red and a position cursor should advance. Press Space again to stop.',
  },
  {
    id: 'p6_home_jumps_to_start',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Home key seeks to 0',
    title: 'Home key resets position to 0',
    instructions: 'Play 5 s in, press Home. The position readout should snap to 0:00.000.',
  },
  {
    id: 'p6_end_jumps_to_last_clip',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'End key seeks to project end',
    title: 'End key seeks to the end of the last clip',
    instructions: 'With at least one clip on an audio track, press End. Cursor should land just past the end of the rightmost clip.',
  },
  {
    id: 'p6_l_toggles_loop',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'L key loop toggle',
    title: 'L toggles looping',
    instructions: 'Press L. Loop button should highlight; ruler should show a loop region. Press L again to disable.',
  },
  {
    id: 'p6_f5_playlist',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'F5 opens playlist',
    title: 'F5 focuses/opens the Playlist (arrangement) panel',
    instructions: 'Press F5. Playlist should come to front and its tab should be highlighted.',
  },
  {
    id: 'p6_f6_channel_rack',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'F6 channel rack',
    title: 'F6 opens/focuses the Channel Rack panel',
    instructions: 'Press F6. Channel Rack should appear / come to front.',
  },
  {
    id: 'p6_f7_piano_roll',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'F7 piano roll',
    title: 'F7 opens/focuses the Piano Roll panel',
    instructions: 'Press F7. Piano Roll should appear / come to front.',
  },
  {
    id: 'p6_f8_browser',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'F8 browser',
    title: 'F8 opens/focuses the Browser panel',
    instructions: 'Press F8. Browser should appear / come to front.',
  },
  {
    id: 'p6_f9_mixer',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'F9 mixer',
    title: 'F9 opens/focuses the Mixer panel',
    instructions: 'Press F9. Mixer should appear / come to front.',
  },
  {
    id: 'p6_ctrl_z_undo',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Ctrl+Z undo hotkey',
    title: 'Ctrl+Z undoes the last action',
    instructions: 'Change a track volume, press Ctrl+Z. Volume fader should snap back.',
  },
  {
    id: 'p6_ctrl_shift_z_redo',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Ctrl+Shift+Z redo hotkey',
    title: 'Ctrl+Shift+Z redoes the last undo',
    instructions: 'After undoing, press Ctrl+Shift+Z. The change should come back.',
  },
  {
    id: 'p6_ctrl_s_save',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Ctrl+S save hotkey',
    title: 'Ctrl+S opens the Save dialog',
    instructions: 'Press Ctrl+S. Native file-save dialog should appear.',
  },
  {
    id: 'p6_ctrl_shift_d_dev_panel',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Ctrl+Shift+D dev panel',
    title: 'Ctrl+Shift+D toggles this panel',
    instructions: 'You are inside the panel already. Close it with the × button, then press Ctrl+Shift+D — panel should reopen.',
  },
  {
    id: 'p6_tap_tempo_button',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'BPM tap tempo',
    title: 'Clicking the tap-tempo button adjusts BPM to tap rate',
    instructions: 'Click the tap-tempo button 4 times at a steady rate. BPM readout should update to approximately your tap rate.',
  },
  {
    id: 'p6_pat_song_toggle_visible',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'PAT/SONG toggle',
    title: 'Pattern/Song toggle visibly switches state',
    instructions: 'Click PAT/SONG button. Its highlight should flip between the two labels.',
  },
)

// ─────────────────────────────────────────────────────────────────────────────
// 6B — Clip drag/resize/split visual behavior
// ─────────────────────────────────────────────────────────────────────────────

PHASE6_TESTS.push(
  {
    id: 'p6_clip_drag_moves_visually',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Clip horizontal drag',
    title: 'Dragging a clip horizontally changes its position',
    instructions: 'Import an audio file, grab a clip by its body, drag right 200 px. Clip should follow cursor and land on snap grid at release.',
  },
  {
    id: 'p6_clip_resize_right_edge',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Clip right-edge resize',
    title: 'Drag right edge of a clip to resize',
    instructions: 'Hover the right edge of a clip — cursor should change to a resize cursor. Drag right to lengthen.',
  },
  {
    id: 'p6_clip_fade_in_handle',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Clip fade-in handle',
    title: 'Drag the top-left fade handle creates a fade-in',
    instructions: 'Grab the small handle at the top-left of a clip. Drag right — a triangular fade-in shape appears inside the waveform.',
  },
  {
    id: 'p6_clip_fade_out_handle',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Clip fade-out handle',
    title: 'Drag the top-right fade handle creates a fade-out',
    instructions: 'Top-right corner — drag left — triangular fade-out appears.',
  },
  {
    id: 'p6_clip_split_scissors',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Clip split at cursor',
    title: 'Split tool cuts a clip at the cursor',
    instructions: 'With the split/scissors tool active, click on a clip. It should divide at that x position into two independent clips.',
  },
  {
    id: 'p6_clip_color_context',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Clip color via context menu',
    title: 'Right-click a clip to change its color',
    instructions: 'Right-click a clip, pick "Color". Clip body should repaint to the selected color.',
  },
  {
    id: 'p6_clip_ctrl_drag_duplicates',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Ctrl+drag duplicates clip',
    title: 'Ctrl+drag copies the clip',
    instructions: 'Hold Ctrl and drag a clip. A new clip should appear at the drop position, original remains.',
  },
  {
    id: 'p6_clip_reverse_context',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Clip reverse',
    title: 'Right-click → Reverse flips the waveform',
    instructions: 'Right-click a clip → Reverse. Waveform mirror flips. Playback plays audio backwards.',
  },
)

// ─────────────────────────────────────────────────────────────────────────────
// 6C — Waveform, meters, visualizations
// ─────────────────────────────────────────────────────────────────────────────

PHASE6_TESTS.push(
  {
    id: 'p6_waveform_renders_on_import',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Waveform render on import',
    title: 'Imported audio clip shows waveform peaks',
    instructions: 'Import an audio file. The clip body should display a black/red waveform matching the file content.',
  },
  {
    id: 'p6_waveform_zoom_redraws',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Waveform zoom correctness',
    title: 'Zooming horizontally re-renders higher resolution peaks',
    instructions: 'Zoom in 5×. Waveform should show more detail (more peaks per pixel), not become pixelated.',
  },
  {
    id: 'p6_master_meter_responds_to_playback',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Master meter response',
    title: 'Master meter bounces with audio',
    instructions: 'Play back a clip. The master L/R meters should move, peak-hold bar should hover above current peak.',
  },
  {
    id: 'p6_track_meter_responds',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Track meter response',
    title: 'Per-track meter moves with its audio',
    instructions: 'Play back a clip on track 1. Track 1\'s meter moves; track 2 (silent) meter stays flat.',
  },
  {
    id: 'p6_oscilloscope_displays',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Oscilloscope',
    title: 'Oscilloscope visualizes master audio',
    instructions: 'Open the oscilloscope. Playing audio should draw a live waveform trace.',
  },
  {
    id: 'p6_spectrum_analyzer_displays',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Spectrum analyzer',
    title: 'Spectrum analyzer shows FFT bars',
    instructions: 'Open the spectrum analyzer. Playing audio should produce moving frequency-domain bars.',
  },
  {
    id: 'p6_clip_indicator_0dbfs',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Master clip indicator',
    title: '0 dBFS clip indicator lights red',
    instructions: 'Play a loud file until master hits 0 dBFS. A red clip indicator should latch on until clicked to reset.',
  },
  {
    id: 'p6_ruler_click_seek_animation',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Ruler click-to-seek',
    title: 'Click on ruler seeks the playhead smoothly',
    instructions: 'Click somewhere on the ruler. Playhead should jump to that x position and a position tick should update in the readout.',
  },
  {
    id: 'p6_loop_region_visible',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Loop region visual',
    title: 'Loop markers visible on ruler when loop on',
    instructions: 'Enable loop. A shaded region with two handles should appear on the ruler.',
  },
  {
    id: 'p6_rms_overlay_on_meter',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'RMS overlay on meter',
    title: 'RMS level shown as secondary bar inside peak meter',
    instructions: 'During loud playback the meter shows a brighter fill for RMS under the peak cap.',
  },
)

// ─────────────────────────────────────────────────────────────────────────────
// 6D — Plugin GUI / mixer / browser
// ─────────────────────────────────────────────────────────────────────────────

PHASE6_TESTS.push(
  {
    id: 'p6_plugin_editor_opens',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Plugin editor window',
    title: 'Plugin editor opens in a floating window',
    instructions: 'Add a VST3/CLAP plugin to a track and click its name / Edit. A native floating window with the plugin GUI should appear.',
  },
  {
    id: 'p6_plugin_editor_closes_cleanly',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Plugin editor close',
    title: 'Closing the plugin window tears down the instance',
    instructions: 'Close the plugin window. No crash; track still has the plugin in its insert chain.',
  },
  {
    id: 'p6_mixer_shows_track_strips',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Mixer strips',
    title: 'Every track has a strip in the mixer',
    instructions: 'Open Mixer. Each track (audio/MIDI/return/automation) should have a labeled strip with fader.',
  },
  {
    id: 'p6_mixer_send_arrows',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Visual send arrows',
    title: 'Sends are shown as arrows between strips',
    instructions: 'Create a send A→B. Open Mixer. Arrow / indicator should connect strip A to strip B.',
  },
  {
    id: 'p6_browser_list_renders',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Browser list',
    title: 'Browser panel lists files from configured paths',
    instructions: 'Open Browser. Sample packs and presets should appear in a tree.',
  },
  {
    id: 'p6_browser_drag_to_track',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Browser drag-to-track',
    title: 'Dragging a sample from browser onto a track creates a clip',
    instructions: 'Drag an audio file from the Browser onto an audio track. A new clip appears at the drop x position.',
  },
  {
    id: 'p6_plugin_sidechain_selector',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Sidechain routing UI',
    title: 'Sidechain source can be selected in plugin inspector',
    instructions: 'For a sidechain-capable plugin, open its wrapper inspector. Pick a source track from the sidechain dropdown. Plugin receives that track as sidechain input.',
  },
  {
    id: 'p6_plugin_wet_dry_knob',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Per-insert wet/dry',
    title: 'Wet knob blends dry and plugin output',
    instructions: 'Place a dramatic FX on a track. Sweep the wet knob from 0 → 100%. You should hear pure dry → pure wet.',
  },
  {
    id: 'p6_fx_chain_bypass',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'FX chain bypass',
    title: 'Bypassing the chain mutes all inserts at once',
    instructions: 'Toggle the chain-bypass switch. All plugins on that track stop processing audio until toggled back.',
  },
)

// ─────────────────────────────────────────────────────────────────────────────
// 6E — Project / chrome / app lifecycle
// ─────────────────────────────────────────────────────────────────────────────

PHASE6_TESTS.push(
  {
    id: 'p6_splash_screen_plays',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Splash screen animation',
    title: 'App splash animates on launch',
    instructions: 'Cold-start the app. Logo slam → glow rings → title fade should play before the main UI appears.',
  },
  {
    id: 'p6_update_modal_triggers',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Update modal',
    title: 'Update modal appears when a newer version is available',
    instructions: 'Launch the app with a prior version installed. Updater should pop a modal offering the new build.',
  },
  {
    id: 'p6_title_bar_fl_menu_order',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'FL-style menu order',
    title: 'Title bar menus: FILE EDIT ADD PATTERNS VIEW OPTIONS TOOLS HELP',
    instructions: 'Read the menus left-to-right — should match the FL Studio ordering exactly.',
  },
  {
    id: 'p6_save_then_dirty_indicator',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Dirty project indicator',
    title: 'Title bar shows unsaved changes after edits',
    instructions: 'Save a project. Make any change. Title bar gains an asterisk / "modified" indicator.',
  },
  {
    id: 'p6_autosave_crash_prompt',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Autosave crash recovery',
    title: 'Autosave prompts after unclean shutdown',
    instructions: 'Kill the process while a project is open. Relaunch — a crash-recovery dialog should offer the latest autosave.',
  },
  {
    id: 'p6_context_menu_on_clip',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Clip context menu',
    title: 'Right-click on a clip shows its context menu',
    instructions: 'Right-click a clip. Menu with Delete / Duplicate / Color / Reverse / Properties appears.',
  },
  {
    id: 'p6_context_menu_on_track',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Track context menu',
    title: 'Right-click on a track header shows its context menu',
    instructions: 'Right-click a track header. Menu with Rename / Color / Delete / Reorder appears.',
  },
  {
    id: 'p6_hint_bar_updates',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Hint bar updates on hover',
    title: 'Hovering a toolbar button updates the hint bar',
    instructions: 'Hover over the Play button. Hint bar in the title row should read "Play / Stop" (or similar).',
  },
  {
    id: 'p6_panels_remember_sizes',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Panel layout persistence',
    title: 'Panel sizes survive restart',
    instructions: 'Resize the Mixer panel, close app, reopen. Mixer should come back at the same size.',
  },
  {
    id: 'p6_keyboard_focus_indicator',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Keyboard focus indicator',
    title: 'Focused controls show a visible outline',
    instructions: 'Tab through UI controls. Each focused element should have a clear focus ring.',
  },
)

// ─────────────────────────────────────────────────────────────────────────────
// 6F — Audio correctness (ears-on)
// ─────────────────────────────────────────────────────────────────────────────

PHASE6_TESTS.push(
  {
    id: 'p6_stereo_pan_audible',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Pan is audible',
    title: 'Full L pan silences right channel',
    instructions: 'Import a mono sample. Set track pan to -1 (hard left). Right channel of master should be silent.',
  },
  {
    id: 'p6_mute_is_audible',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Mute button audible',
    title: 'Muted track does not play',
    instructions: 'Playback a loud clip, click mute. Audio stops for that track.',
  },
  {
    id: 'p6_solo_is_audible',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Solo button audible',
    title: 'Soloing a track silences others',
    instructions: 'Two tracks playing. Solo track 1 — track 2 goes silent.',
  },
  {
    id: 'p6_pitch_semitones_audible',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Track pitch semitones audible',
    title: '+12 semitones doubles frequency',
    instructions: 'Play a tonal sample. Set pitch +12. It should sound one octave higher.',
  },
  {
    id: 'p6_fine_tune_audible',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Fine tune audible',
    title: '+100 cents equals +1 semitone shift',
    instructions: 'Set fine tune to +100 cents. Should sound identical to +1 semitone.',
  },
  {
    id: 'p6_phase_invert_audible',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Phase invert audible',
    title: 'Phase invert flips waveform polarity',
    instructions: 'Play the same sample on two identical tracks, invert phase on one. Output should cancel to near silence.',
  },
  {
    id: 'p6_swap_lr_audible',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Swap L/R audible',
    title: 'Swap L/R puts left content on right channel',
    instructions: 'Play a sample that is clearly panned left. Enable swap L/R — sound should move to the right speaker.',
  },
  {
    id: 'p6_filter_lp_audible',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Filter LP audible',
    title: 'Low-pass filter removes highs as cutoff drops',
    instructions: 'Set filter type LP, sweep cutoff from 20k → 200 Hz. Audio should progressively lose brightness.',
  },
  {
    id: 'p6_filter_hp_audible',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Filter HP audible',
    title: 'High-pass filter removes lows as cutoff rises',
    instructions: 'Set HP, sweep cutoff 20 → 5000 Hz. Low end disappears progressively.',
  },
  {
    id: 'p6_loop_audible',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Loop region audible',
    title: 'Playback loops between markers',
    instructions: 'Set loop start/end on the ruler, enable loop, play. When position hits end it should jump back to start.',
  },
  {
    id: 'p6_precount_click',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Precount before record',
    title: 'Precount plays a count-in click before recording',
    instructions: 'Arm a track, hit record. A metronome count-in should play for N bars, then recording starts.',
  },
  {
    id: 'p6_metronome_click',
    kind: 'MANUAL',
    phase: 6,
    phase1Item: 'Metronome click',
    title: 'Metronome click audible during playback',
    instructions: 'Enable metronome, play. A click should sound on every beat aligned to BPM and time signature.',
  },
)
