### New features
- feat: takes go to disk while you record, with no length limit; they were held in memory and stopped at 20 minutes
- feat: takes can be saved as 24-bit or 32-bit float, in the song's Recordings folder or one folder you pick (Audio settings, Recording)
- feat: monitoring Auto hears the input while stopped or recording and the track itself while playing back; the mixer's IN button cycles Off, In and Auto
- feat: a track can record the master or a bus in real time (Records, in the channel's routing), to resample the song

### Bug fixes
- fix: monitoring an armed track is heard, stopped and playing, also on a track with no plug-ins; it was silent unless direct monitoring was on

### Improvements
- chore(deps): tokio 1.54.1 for RUSTSEC-2026-0386
- fix(project): the new track fields go last, so older songs keep opening
- feat(recording): record the master or a bus in real time
- feat(recording): monitoring that is heard, with Off, In and Auto
- feat(recording): choose 24-bit or 32-bit float takes, and their folder
- feat(recording): takes go straight to disk, with no length limit
