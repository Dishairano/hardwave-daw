### New features
- feat: smart disable: a plug-in with nothing to do (a reverb on an empty track, a synth between its notes) stops using the processor until sound, notes or a knob move reach it (a knob moved in the plug-in's own window included), and what you hear stays the same. It is on by default in Settings > Audio for the built-in plug-ins; other plug-ins are only put to sleep when they report that they have finished. Tick or untick "Allow smart disable" in a plug-in's slot menu to decide for that plug-in yourself.
- feat: the multi-core audio setting hands work to its helper threads without making the audio wait, and keeps the light parts of a song on one core, so switching it on no longer costs small songs anything

### Bug fixes
- fix: with audio worker threads switched on, a big song no longer crashes the DAW during playback when one of the helper threads falls behind
- fix: changing the number of audio worker threads, or a plug-in failing on one of them, no longer freezes the DAW
- fix: moving a knob in a VST3 or CLAP plug-in's own window no longer makes the audio wait for the window, and the last bit of a knob turn is no longer lost
- fix: stopping, moving the playhead or previewing a sample while other samples are still loading no longer makes the audio wait for them
- fix: odd-sized audio buffers from the sound card cost less processor time, and a delay-compensated input no longer slips out of time when one arrives
- fix: stretch, warp and tuning processing that runs while the song plays now runs below normal priority, so it never takes the processor away from the audio or the window
- fix: the mixer meters no longer hold up the engine while they are read, so commands and the audio no longer wait behind them 30 times a second
- fix: the freeze log (written when the window stops responding) no longer pauses the audio threads (WASAPI, exclusive mode, ASIO or the worker threads) to read them, so writing it cannot interrupt the sound
- fix: when a sample preview ends, its memory is no longer freed on the audio thread, where it could cause a short dropout
- fix: a clip whose sample was pushed out of the sample cache keeps playing after a stop or a jump instead of going silent
- fix: a CLAP plug-in that sends more than 1024 parameter changes at once no longer loses the rest; they arrive with the next audio buffer
- fix: a CLAP plug-in's own window no longer has its parameter changes handed over at the same moment the audio runs, which the CLAP standard forbids and which can crash a plug-in
- fix: a CLAP plug-in can no longer read past the end of its input when it is given a short input or none
- fix: CLAP plug-in knobs can be automated when the plug-in marks them as automatable; before, only stepped switches could
- fix: knob changes that a VST3 or CLAP plug-in reports while it plays now reach automation recording instead of being thrown away
- fix: a knob move in a plug-in window is no longer left out of automation recording when it happens just as the DAW collects the recorded moves
- fix: when automation and a knob move in a VST3 window land at the same moment, the newer one now wins; before, an older automation value could undo your move
- fix: recording automation no longer records the automation that is playing back as new knob moves when a plug-in reports the value back
- fix: knob moves made just before loading a preset or a saved state no longer land on top of the loaded preset
- fix: the built-in Flanger's Invert switch now flips the feedback for the hollow, inverted flanger sound; before, turning Invert on just switched the feedback off
- fix: turning a knob on the built-in Chorus, by hand or with automation, no longer makes its sweep jump back to the start
- fix: the built-in Auto-Filter no longer cuts the first few samples of the first sound through it after it is added or the audio restarts
- fix: if your audio device drops out (a remote desktop disconnect, an unplugged USB interface, a driver reset), your plug-in settings stay as you left them; before, every knob turned since the last save jumped back when the audio came back
- fix: when the audio device drops out or you change an audio setting (buffer size, sample rate, output device), plug-ins are no longer asked for their settings on the audio thread, which could crash or freeze some VST3 and CLAP plug-ins; their knobs still come back as you left them
- fix: switching the audio driver (for example from WASAPI to ASIO) no longer resets plug-in knobs turned since the last save

### Improvements
- improve: updates are now signed with a new signing key. A DAW on v0.266.46 or older first updates to v0.266.47 and then to this version, with no reinstall
- improve: an installer (HardwaveDAW-Setup.exe) downloaded before v0.266.48 can no longer install the DAW; download a new one
