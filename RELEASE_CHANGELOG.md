### Bug fixes
- fix: a MIDI clip made shorter no longer plays the notes past its end, and splitting a MIDI clip no longer plays everything after the split twice; splitting a resized audio clip now starts the right half at the right place in the sample
- fix: cut, copy and paste work across tracks; paste used to fail on any track but the clip's own, and cut then paste pasted nothing
- fix: lining a track up with another no longer moves it further away; a late track now waits less, or the other track waits for it
- fix: clicking a track's name or an empty spot in its row selects the track again, so a paste lands there

### Improvements
- fix(playlist): clicking a track picks it again, so a paste lands there
- fix(mixer): lining a track up with another no longer moves it the wrong way
- fix(playlist): cut, copy and paste work across tracks
- fix(playlist): a shortened MIDI clip stops at its end, and a split plays each note once
