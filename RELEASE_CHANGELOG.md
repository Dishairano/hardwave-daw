### New features
- feat: every armed track records its own input (Records, in the channel's routing): a singer on input 1 and a guitar on input 2 at once, each with its own take
- feat: song sections can be moved, renamed and repeated from a right-click on their band in the ruler
- feat: the built-in synths have Poly, Mono and Legato with glide and a voice count
- feat: slices go straight to the playlist or to channels from the slicer
- feat: File > Export the whole song as one MIDI file
- feat: previews at the song's tempo keep their pitch, and start at once
- feat: your own click sounds, played by the engine on the beat (Settings, Recording)
- feat: the audio editor saves next to the original file, and into the song
- feat: follow MIDI timecode from a video app or another DAW (Settings, MIDI), with the timecode the song starts at
- feat: with others in an Ableton Link session, play starts on the session's beat and stays on it

### Bug fixes
- fix: picking, muting or soloing a track no longer clicks in the sound; each one rebuilt the audio engine for longer than a sound card block (14 ms on a new song, now 0.3 ms)
- fix: a synth track no longer allocates memory on the audio thread while it plays, another cause of clicks that come and go
- fix: arming a track opens the audio input; it only ran while the monitor switch in Audio settings was on, so a take could come out silent
- fix: when the audio input will not open, arming and recording say why, with the Windows setting to change, instead of recording silence
- fix: channel rack step edits are undo steps
- fix: a sound card that delivers a bigger block than asked plays it whole; the rest was silence
- fix: MIDI clock out sends the song position, so a drum machine starts where the song is, not from the top
- fix: following a MIDI clock, Continue carries on from the song position the master sent

### Improvements
- fix(audio): the audio thread allocates nothing while a song plays, and a test keeps it so
- fix(audio): clicking a track no longer clicks in the sound
- feat(link): play starts on the session's beat and stays on it
- fix(recording): an input that will not open says why, before a silent take
- feat(midi): follow MIDI timecode, and a clock master's song position
- fix(midi): the clock out follows the song, with its position
- fix(recording): arming a track opens the interface's input
- feat(recording): every armed track records its own input
- feat(sections): rename, move and repeat a section from the ruler
- feat(sections): move and rename song sections
- feat(slicer): slices straight to the playlist or to channels
- perf(browser): a tempo-matched preview starts at once
- feat(synths): Poly, Mono and Legato with glide and a voice count
- feat(export): the whole song as one MIDI file
- feat(browser): previews at the song's tempo keep their pitch
- fix(channel-rack): step edits are undo steps
- feat(audio-editor): save next to the original, and into the song
- feat(metronome): your own click sounds, played by the engine on the beat
- fix(audio): a driver's bigger-than-asked block plays whole
