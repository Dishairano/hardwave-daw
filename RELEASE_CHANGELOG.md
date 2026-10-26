### New features
- feat: modulation from another track's level (a kick ducking a bass, with a release time) or from a MIDI controller such as the mod wheel, as well as from an LFO
- feat: modulation moves a track's volume, pan and sends, as well as a plug-in's knobs, on top of the fader and the automation
- feat: MIDI clips in the clip launcher: right-click a clip on the playlist, Send to clip launcher, and it loops in its track's row, in time and on the exact sample, with the built-in synth or a plug-in instrument. Audio clips from the playlist go in the same way
- feat: a plug-in running in its own process (Own process, and every 32-bit plug-in) now has its knobs in the DAW, plays notes (an instrument works), follows the song's tempo, and opens its own window, beside the DAW; knobs moved in that window are recorded as automation and saved with the song
- feat: a warped clip keeps its pitch: the pieces between warp markers are stretched to fit instead of played faster or slower like tape (Resample on the clip still plays it like tape)
- feat: a note's own pan and fine tuning reach a plug-in instrument (CLAP and VST3, as note expression), not only the built-in synth
- feat: a macro can turn a send
- feat: File > Export DAWproject writes the song in the open format Bitwig Studio, Studio One and Cubase read: tracks, notes, audio clips with their samples, the mix, sends and routing, VST3 and CLAP plug-ins with their settings, automation and tempo changes
- feat: File > Open DAWproject opens a song from Bitwig Studio, Studio One or Cubase, with its samples unpacked next to it and its plug-ins found on this computer

### Bug fixes
- fix: the convolution reverb could overload playback: it multiplied every sample by the whole impulse, about eight billion operations a second for the default plate. It now uses partitioned FFT convolution and sounds the same, and picking another space while playing no longer builds the impulse on the audio thread
- fix: MIDI from a keyboard or controller could be lost: the thread that applies MIDI mappings took events from the same queue as the audio thread, so a note or a knob move sometimes never reached the track. Each now gets every event, and a control surface's own buttons no longer play an armed instrument
- fix: a transposed audio clip played flat when shifted up (38 cents for a one-second sample raised an octave, more on a one-shot), and a stretched clip's sound sat up to 21 ms early
- fix: a CLAP instrument now gets pitch bend, channel pressure and poly aftertouch (they were dropped), and hosting a CLAP plug-in no longer allocates on the audio thread every block
- fix: a plug-in killed or crashed in its own process is noticed and started again, and each one keeps a short log of its own for support

### Improvements
- fix(sandbox): the plug-in's own window opens from its process, and its life is logged
- feat(import): File > Open DAWproject, a song from Bitwig Studio, Studio One or Cubase
- feat(export): File > Export DAWproject, for Bitwig Studio, Studio One and Cubase
- fix(sandbox): a killed plug-in process is noticed, and its window thread has COM
- feat(macros): a macro can turn a send
- feat(midi): a note's own pan and tuning reach plug-in instruments; CLAP gets bends and pressure
- feat(warp): a warped clip keeps its pitch
- fix(dsp): a phase-vocoder pitch shift lands on the note, and a stretch puts sound where it belongs
- fix(midi): the audio thread and the mapping thread each get every MIDI event
- feat(sandbox): a plug-in in its own process has its knobs, its notes and its own window
- feat(launcher): MIDI clips and playlist clips in the clip launcher
- feat(modulation): a track's level or a MIDI controller as a source, and volume, pan and sends as targets
- test(realtime): every built-in plug-in is checked for allocations on the audio thread
- fix(reverb): the convolution reverb plays in real time, and switches space without a stall
- test: every built-in plug-in is checked for allocations on the audio thread in every build
