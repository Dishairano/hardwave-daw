### Bug fixes
- fix: with the song stopped, a key or pad held longer than ten seconds went silent while still held, and came back as a release tail when let go. Keys held down and the sustain pedal now keep it sounding
- fix: on an MPE track, a slide on an MPE controller moved the built-in synth only a fraction of the way, and a plug-in instrument could play a note's detune at a fraction too. Member channels now bend over the 48 semitones the MPE spec sets, and the instrument is told so, also when keys are played with the song stopped
- fix: capture what you just played came back with "nothing that makes a note" after a chord held longer than ten seconds; it now captures the chord and what led up to it
- fix: a key still down when capture was pressed came out one tick long; it now lasts to the end of the captured part

### Improvements
- With the song stopped, the engine ran for ten seconds after the last
- An MPE member channel's pitch bend covers 48 semitones, as the MPE
- Capture takes the playing since the last pause of ten seconds; a key
- A key still down when capture is pressed lasts to the end of the take;
