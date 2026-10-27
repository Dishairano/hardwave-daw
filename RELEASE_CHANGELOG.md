### New features
- feat: a track's MIDI effects now act on what you play live into a plug-in instrument. A chord, scale or transpose effect used to change only the built-in synth; a plug-in got the keys as played
- feat: the arpeggiator in a track's MIDI effects plays live: hold a chord and it steps through it at the song's tempo, starting at once, up, down, up-down or as played, over one to four octaves. It worked only on clip notes before
- feat: the History panel lists the real undo steps. It kept a list of its own, which missed a note drawn in the piano roll, a fader moved on a desk and many other edits, so clicking a row could undo a different number of steps than it said. Every step is there now, with what it did ("Add 8 notes: Lead", "Volume: Bass", "Delete track Kick"), in English or Dutch

### Bug fixes
- fix: letting go of a key ends every note the MIDI effects started from it, also across an edit to the song while the key is held, so nothing hangs
- fix: the control surface switch (Settings, MIDI) is kept between launches; a desk went dead at every start until it was switched on again
- fix: the control surface caption said the scribble strips and LED rings were not driven; they have been since v0.266.23
- fix: freezing or exporting a track with a plug-in in its own process left an unused copy of that plug-in's state behind for the rest of the session

### Improvements
- fix(history): a long gesture's name is not dropped
- feat(midi fx): a track's MIDI effects act on a live keyboard for plug-in instruments, arpeggiator included
- chore(dev): the screenshot harness answers get_undo_history
- feat(history): the History panel lists the real undo steps
- chore(permissions): set_control_surface_protocol's generated permission file
- fix(sandbox): a plug-in process nothing uses is forgotten
- fix(control surface): the switch and the protocol are kept between launches
- fix(settings): the control surface caption says what the desk does now
- feat(control surface): HUI desks, beside Mackie Control
