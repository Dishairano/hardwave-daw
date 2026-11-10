### New features
- feat: live rooms: two producers on one song. Tempo changes, the channel rack, markers, arrangements, macros and VCAs are shared, each part on its own, so two people editing at the same moment both keep their change
- feat: Settings, MIDI: with Control surface on, pick the desk's protocol (Mackie Control or HUI) and its MIDI input; a desk found by its name (X-Touch, Mackie, iCON, FaderPort and others) is picked by itself
- feat: Apply in Settings > Audio plays a short test tone on the device, left then right, and names it; Test sound plays it again
- feat: a MIDI controller plugged in later turns on by itself, with a message and Turn off (remembered per device); a control surface is offered as one instead
- feat: one first-run window: start screen, sound card with a test tone, your controller (only when found), then the Hardstyle starter; it no longer comes back after New project

### Bug fixes
- fix: MP3 export uses a new encoder; VBR is real VBR now and exports play at exactly the rendered length
- fix: a song taken from a room plays at its own tempo, the window shows it, and Ctrl+Z can no longer send your old song into the other person's song
- fix: in a room, an edit that loses to a newer one is sent back so both songs end the same, and a tempo or marker change right after taking the song reaches the other side
- fix: the other person's new track shows up in your playlist and mixer, and their cursor goes when they leave
- fix: a song too large for a room is not sent and the host is told why, instead of the room closing; songs up to about 12 MB now fit
- fix: a change to hundreds of tracks at once reaches the other person whole, the room panel counts what it receives, and a first-time guest is told to get the song
- fix: deleting several selected clips at once is one undo step, and one undo brings them all back
- fix: clicking a clip no longer adds an undo step that changes nothing; a resize, a group move, a move onto another track and a fade-handle drag are each one undo step
- fix: moving or resizing clips, ripple moves, warp and unwarp, consolidate, audio to MIDI, send to launcher, section and insert/delete time edits, and markers now mark the song as changed, so closing asks to save and autosave runs
- fix: insert time, delete time and moving or repeating a section move the markers on screen too
- fix: splitting a clip no longer puts a fade-out and fade-in dip at the cut (comping had the same dip)
- fix: a fade on a clip after a tempo change lasts as long as it is drawn
- fix: a reversed clip plays the right audio when it is repitched in resample mode, sits on a pitched track, or is bounced at another sample rate
- fix: dragging a clip to another track never deletes it
- fix: an audio clip on a MIDI channel can be copied and pasted
- fix: with Control surface on, only the desk's own MIDI input is read as the desk: a keyboard next to it no longer arms, solos or mutes tracks with its low notes, presses Stop/Play/Record with G6 to B6, or moves fader 1 with its pitch wheel
- fix: a sample clicked in the browser plays while the song is stopped (it was only heard while the song played)
- fix: a synth played on the keyboard with the song stopped is heard through its bus, reverb return and sends
- fix: sound routed or sent to an instrument track is heard instead of disappearing
- fix: scrolling far along the mixer shows the channels instead of an empty area
- fix: a bounced, frozen or consolidated part plays back as the part instead of as loud distortion (the file said integer samples while it held float ones), no longer comes back quieter, and no longer gets the master fader a second time; parts bounced with earlier builds need bouncing again
- fix: a frozen track plays as it did live (its fader, pan, automation, VCA and channel settings were applied twice)
- fix: with the reference switched on, an export, stem, freeze or bounce is the mix, not the reference record
- fix: exports no longer run a frozen track's plug-ins again on top of its render (effects were doubled, a frozen instrument exported silent)
- fix: stem export makes files only for tracks that play something (it rendered all 500 empty inserts of a new song)
- fix: a track routed into a bus has sound in its stem, every stem keeps its send reverb, and the stems add up to the mix
- fix: freezing a track again replaces its old render instead of playing both
- fix: bouncing or freezing a muted track, or while another track is soloed, renders the part instead of silence
- fix: a loop or range export's tail holds only reverb, delay and release, not the next bar's music
- fix: in a song with a tempo change, a frozen or bounced part plays to the end of the song
- fix: keys or controller moves made during an export stay out of the file
- fix: dither works for 16-bit FLAC exports

### Improvements
- fix(tests): live MIDI smoke tests play through the live path again
- security: every release now comes with a signed Secure Code Check report, a component list (SBOM) and its vulnerability history, at hardwavestudios.com/security/releases; a failed check stops the release
- security: the updater, sample packs, DAWproject and FL zip packages use the fixed zip library, and the anyhow and rand advisories are closed: no open advisories in shipped code
