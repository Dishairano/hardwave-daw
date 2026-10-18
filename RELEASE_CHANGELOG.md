### New features
- feat: Sample window: double-click an audio clip, or open a sampler channel, for the channel settings FL has: volume, pan, pitch, stretch, fades, normalize, DC offset, reverse, polarity, swap stereo, sample start and length, trim, fade stereo, declicking. Sampler channels add the volume envelope, offset, loop with crossfade, mono, cut itself, porta and root note. Every change is one undo step.

### Bug fixes
- fix: the built-in kick played one buffer late (21 ms at 1024), in playback and in renders.
- fix: a consolidated part or a sample dropped on a MIDI channel was silent.
- fix: a bounce, freeze or consolidate started late by the plug-ins' latency and ran through the master twice.
- fix: with no output chosen, the DAW played through a monitor's HDMI audio while a real output was there.
- fix: a plug-in window opened after the audio device dropped out showed default settings.
- fix: security update of the TLS library (rustls 0.23.46, RUSTSEC-2026-0346 to 0349).

### Improvements
- improve: Consolidate shows in Edit > History; sampler settings are saved with the project; the channel rack lists its instruments on a fresh start; the app has its icon.
