### Bug fixes
- fix: the MIDI light and the scopes no longer wait for the engine, and the working-time counter no longer runs on the window's thread
- fix: a MIDI port scan no longer holds up the engine; on a busy computer it froze the DAW for seconds at a time

### Improvements
- fix(midi): no one waits on the engine for a MIDI port scan
- diag(watchdog): log the other threads when the main thread is stuck
- fix(engine): the scope and MIDI-light polls no longer wait on the engine
- improve: when the DAW stops answering, its log also names what its other threads are doing
