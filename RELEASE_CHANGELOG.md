### Bug fixes
- fix: a key let go while its track was muted never reached a plug-in instrument, so the note sounded again on unmuting with nobody holding it. The release is now sent when the track plays again
- fix: with the song stopped, live keys could miss a plug-in instrument (on a kick-synth track, or a release after the built-in voices had died out)
- fix: changing a track's MIDI effects while keys were held stopped the arpeggio and silenced a held chord until the next key press; the keys now play on through the new effects
- fix: the History panel could put an edit's name on the step before it, when the edit took no undo step of its own. Names now only go to a step their own command took
- fix: a clip dragged to another track showed in the History panel as a delete on the track it left; it reads "Move clip to" the new track
- fix: past 256 undo steps, the History panel's first row said "Project opened"; it says "Oldest step kept"
- fix: the History panel no longer holds up other edits while it is open
- fix: on a Mackie desk, the master fader's motor kept moving under a hand on it; it now waits, as the strips' motors do

### Improvements
- fix(midi): a release while muted reaches a plug-in after a rebuild too
- fix(control surface): the master fader's motor leaves a hand on it alone
- The History panel's list is worked out under the song's and the
- A clip dragged to another track read as "Delete clip" on the track it
- Past the history's cap of 256 steps, the panel's first row said
- fix(midi fx): a chain changed under held keys plays them through the new one
- fix(midi): live keys are not lost while a track is muted or the song is stopped
- test(engine): the capture clock test allows a sample of wall-clock jitter
