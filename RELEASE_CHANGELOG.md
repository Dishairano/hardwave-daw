### New features
- feat: plug-ins from other makers (VST3 and CLAP) now run in a process of their own by default. When one crashes or stops answering, the DAW stays open, the song keeps playing, nothing unsaved is lost, a message says which plug-in it was, and it is started again with the settings it had. Hardwave's own plug-ins and the built-in ones still run inside the DAW. It costs one audio buffer of delay per such plug-in, which delay compensation lines up again, and the plug-in's window opens on its own beside the DAW. To run a plug-in inside the DAW again, open its slot menu and switch "Own process" off

### Bug fixes
- fix: a knob of a plug-in in its own process that you set from the DAW (the parameter sheet) while the song is stopped is now kept: it reads back right away and is in the song when you save straight after. Before, it could fall back to its old value
- fix: after a plug-in crash, the plug-in comes back with the knobs as you left them, not as they were at the last save
- fix: a plug-in that changes its delay while the song plays (a limiter's lookahead knob) is lined up with the other tracks again within half a second; before, it stayed out of time until the next edit
- fix: the threads that carry audio to and from a plug-in's own process run at the audio priority, so a busy computer does not cause gaps in that plug-in's sound

### Improvements
- improve: a plug-in in its own process is heard exactly one buffer later, always. Before, one slow moment could leave its track a buffer further behind the others for the rest of the session
- improve: a plug-in in its own process now gets the sidechain, reports its own delay to delay compensation, lists its own presets, and can be saved as a .vstpreset
- improve: exporting, bouncing and freezing wait for a plug-in in its own process, so it renders every sample and adds no delay; if such a plug-in stops during a render you are told, and the file with a gap in it is not kept
