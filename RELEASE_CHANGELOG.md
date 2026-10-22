### Bug fixes
- fix: Add tempo change and Add time signature change in the ruler change the song itself; they only drew a flag
- fix: delay compensation includes a plug-in added while the song plays, CLAP plug-ins report their latency, and the Plugin delay compensation switch works
- fix: the control-surface setting and the Own process switch no longer promise HUI and a parameter sheet that are not there

### Improvements
- fix(ui): two settings no longer promise what they cannot do
- fix(engine): delay compensation follows plug-ins as they come and go, and its switch works
- fix(playlist): the ruler's tempo and time signature changes change the song
