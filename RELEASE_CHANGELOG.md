### New features
- feat: View > Zoom to selection, zoom presets of 4, 16 and 64 bars, and View > Follow a page at a time
- feat: Ctrl+Left/Right nudges every selected clip, Ctrl+Shift by one tick, and Alt held while dragging moves a clip off the grid
- feat: the timeline can count samples, and timecode runs at 24, 25 or 30 fps (the MIDI timecode sent out too)
- feat: the mixer strips show RMS inside the peak, and the master shows its left/right correlation
- feat: Tape, Exciter and Master get Off/2x/4x oversampling, and the Saturator now keeps its oversampling setting
- feat: a knob learned to a MIDI controller is recorded while writing automation
- feat: mixer snapshots keep width, sends and which plug-ins are on

### Bug fixes
- fix: a reference track is measured like the mix, so matching its loudness is no longer a quarter dB off

### Improvements
- chore(permissions): the generated permission files for drain_midi_mapped_moves
- feat(mixer): snapshots keep width, sends and which plug-ins are on
- feat(automation): a knob learned to a MIDI controller is recorded while writing
- feat(playlist): a samples ruler, and timecode at 24, 25 or 30 fps
- feat(mixer): RMS inside the strip meters, and a correlation bar on the master
- feat(playlist): zoom to the selection, zoom presets, and a page-at-a-time follow
- feat(playlist): nudge every selected clip, nudge by one tick, and Alt-drag off the grid
- fix(reference): a reference track is measured by the same meter as the mix
