### Bug fixes
- fix: in an FL project with several arrangements, a playlist row switched off in the arrangement open in FL stayed silent in every other arrangement after switching to it. Each arrangement now keeps its own switch
- fix: an FL pattern clip trimmed at its start dropped a note held over where it begins; the note now plays its tail from there, as in FL
- fix: an FL mixer insert going on to the master below full while also sending to a reverb or group played at full; it now plays at its level, with its sends where FL had them
- fix: an FL project cut off part-way (an interrupted copy) was refused whole; the part that is there now opens

### Improvements
- A playlist row switched off in the arrangement open in FL muted the
- A note held over the start of a pattern clip trimmed there was
- A route on to the master below full beside a send (a group at -6 dB
- A file cut off in the middle of an event was refused whole; what
