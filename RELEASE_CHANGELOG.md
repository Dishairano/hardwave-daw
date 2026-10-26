### New features
- feat: a Mackie Control desk follows the mix: motor faders, mute, solo and arm lights, V-pot pan rings, and track names and levels on the scribble strips
- feat: scripts can read the song: its tempo, every track, its clips and their notes
- feat: mixer snapshots keep each plug-in's own settings, and recalling one reloads only the plug-ins that changed
- feat: a MIDI take can replace the notes it is played over (Settings, Recording), instead of landing on top as a second clip that plays as well

### Bug fixes
- fix: when the sound card will not open, a banner says why in words and offers Retry audio and Audio settings; a toast used to vanish with a raw Windows code
- fix: a sound card that sends shorter blocks than asked no longer crackles in monitoring or writes silence into the take
- fix: a note held on the keyboard is let go when the keyboard moves to another track; it hung before
- fix: a take lands where it was played after a Link start, a timecode locate or a jump; it could come back as a stub plus the real take at bar 1
- fix: arming, disarming and undo wait until the recording stops, instead of mixing up the takes
- fix: following a MIDI clock, a jump keeps playing; following timecode, a pre-roll waits at the top; while Link holds the start, the clock out stays quiet
- fix: cancelling a long take is instant; it read the whole take into memory first, about 2.8 GB for an hour
- fix: a full disk or a pulled drive during a take keeps what was recorded and says the take is incomplete, and temporary take files left by a crash are cleaned up
- fix: a Mackie desk's lights and faders go to the desk only, not to a synth on another MIDI output
- fix: a loop jumps back on the exact sample; each pass ran up to a block long, and drifted against a clock or Link
- fix: the old mixer (Options) no longer freezes for seconds when it opens on a big song, or drops frames while the song plays
- fix: playing out of outputs 3/4 or further of an interface gives sound; it was silent

### Improvements
- fix(audio): playing out of outputs 3/4 or further gives sound
- feat(recording): a MIDI take can replace the notes it is played over
- feat(mixer): snapshots keep each plug-in's own settings
- fix(mixer): the old mixer no longer stalls on a big song or redraws every strip per meter tick
- fix(engine): a loop jumps back on the exact sample
- fix(control-surface): desk feedback goes to the desk, and only what changed on it
- fix(recording): a disk error keeps what was recorded, and old take files are cleaned up
- fix(recording): cancelling a take does not read it into memory first
- Following a MIDI clock master, a jump arrives as Stop, the song
- Following MIDI timecode with the song starting at 01:00:00 and the
- While a Link session holds the start of playback, MIDI clock out sent
- fix(recording): arming or undoing during a recording no longer mixes up the takes
- fix(recording): a take lands where it was played after a Link start or a jump
- fix(midi): a note held when the keyboard moves to another track is let go
- fix(recording): a short audio block records its own length, not the full buffer
- fix(audio): a sound card that will not open says why, and stays on screen
- fix(scripts): the script runner without a song is for tests only
- feat(scripts): a script can read the song
- feat(control-surface): scribble strips, V-pots, lights and motor faders that follow the mix
- perf(playlist): an edit to one track reloads that track, not the song
- perf(playlist): the track column draws only the rows in view
- perf: the track column beside the playlist draws only the rows in view, so a song with hundreds of tracks stays quick to work in
- perf: moving, trimming or muting a clip reloads that track, not every track in the song
