### New features
- feat: punch recording works for MIDI as well as audio

### Bug fixes
- fix: recorded MIDI sits where you played it. Notes used to be pulled onto the nearest audio block, which at a 512 sample buffer is about 10 ms late, every time
- fix: recording MIDI over a loop keeps every pass as its own clip, the last one playing and the earlier ones muted. The take used to be discarded completely
- fix: a MIDI recording can be undone
- fix: recorded MIDI lands in the right bar in a song with tempo changes

### Improvements
- improve: a MIDI take that captured nothing says so instead of failing quietly
