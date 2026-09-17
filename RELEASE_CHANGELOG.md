### New features
- feat: "Play truncated notes" works. Start playing in the middle of a long note and it plays from there, with its envelope already open, instead of re-attacking or staying silent
- feat: the velocity curve you pick in the setup wizard now changes how hard your controller plays, per controller, and is remembered between sessions
- feat: "Enable MIDI remote control" now really stops MIDI input when you switch it off, and resumes instantly when you switch it back on

### Bug fixes
- fix: kick tracks no longer make the audio engine ask the system for memory on every block, which is a cause of clicks at small buffer sizes
- fix: a held note no longer keeps sounding after you press stop or move the playhead, and the setting that promised this now works
- fix: both audio behaviour settings survive a restart. They were saved but never reached the engine, so they only applied in the session you set them
- fix: the setup wizard now really checks whether MIDI works on your machine instead of always showing a green tick
- fix: the curve drawn in the wizard now matches what the curve does to your playing

### Improvements
- improve: removed the controller-type dropdown and the "Custom" curve. Neither did anything, and a control that does nothing is worse than no control
