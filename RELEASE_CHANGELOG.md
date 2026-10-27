### Bug fixes
- fix: a note drawn in the piano roll stayed silent, and a moved or deleted note kept playing where it was, until something else changed the song (adding a track, for one). Drawing, moving, resizing and deleting notes is now heard at once
- fix: on an instrument (MIDI) track the mixer's polarity, left/right swap and track delay, and the channel rack's filter (cutoff and resonance), did nothing. They now act on instrument tracks as on audio tracks, after the instrument and its effects and before the width and the fader
- fix: unfreezing a track brought it back without its plug-ins: its effects and instrument were gone until the song was opened again. They are kept while the track is frozen and come back on unfreezing, with their settings; saving a song with a frozen track keeps them too

### Improvements
- fix(piano roll): a note drawn, moved or deleted is heard at once
- fix(engine): unfreezing a track gives its plug-ins back
- fix(engine): polarity, swap, delay and filter act on instrument tracks
- chore(fl-import): flp_structure, an FL project's structure as FL has it and as imported
