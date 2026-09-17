### Bug fixes
- fix: recordings are saved next to your project instead of the Windows temp folder, where they could be deleted without warning
- fix: a take now lands at the position you recorded it, instead of always at bar 1
- fix: arming a track is enough to record it. With input monitoring off, recording used to capture silence and say nothing
- fix: a recording no longer loses short gaps of audio, and no longer asks the audio engine for memory while it runs

### Improvements
- improve: if a take captured nothing, captured only silence, or hit the twenty minute limit, Hardwave says which instead of leaving you to work it out
- improve: takes are named by date and time, so a Recordings folder is readable
