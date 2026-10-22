### New features
- feat: Tools > Mix check renders the song and places its drop against 33,605 harder-styles releases; overs and a left/right imbalance are flagged
- feat: VCA faders: one fader that moves a group of tracks, in a column of their own in the mixer
- feat: clip colours are saved with the song, and clips can be renamed from the clip menu
- feat: the export notice says how loud the file is (LUFS and true peak) and warns about a true peak over 0 dB
- feat: an option to free the sound card while another program is in front, for ASIO drivers that serve one program at a time (off by default)

### Bug fixes
- fix: the meters' true peak now catches the overs between samples, integrated LUFS is measured, and LUFS normalizing lands on its target (it was a quarter LU off)
- fix: a plug-in's window shows the instance that plays; LoudLab's window waited for the audio engine forever
- fix: plug-in instruments play a note the playhead starts inside (with play truncated notes on) and let go of notes the playhead jumps past
- fix: exports flush denormals too, so quiet reverb tails no longer slow an export down

### Improvements
- feat(export): the export notice says how loud the file came out
- fix(metering): a real true peak, integrated LUFS that is filled, and the standard's K-filter
- fix(midi): plug-in instruments get held notes after a jump, and lose them after one
- chore(permissions): add the generated permission files for the newer commands
- feat(playlist): clip colours are saved with the song, and clips can be renamed
- fix(audio): freeing the sound card in the background is off by default
- fix(mixer): VCA faders in a fixed column, not after 500 channels
-4.8). The low end and the crest factor are shown as numbers with no
- chore(ui): remove the inspector panel, which was shown nowhere
- fix(engine): exports flush denormals too
- fix(audio): the DAW frees the sound card while another program is in front
- feat(mixer): VCA faders
- fix(plugins): a plug-in's window shows the instance that plays
