### Bug fixes
- fix: renaming a channel or insert also renames its playlist track when that track still has the channel's old name (as in a new song, a starter or an older song); a playlist track you named yourself keeps its name
- fix: the browser's Plug-ins tab lists your plug-ins and the built-in ones as soon as you open it, without pressing Scan
- fix: on Windows, sample paths no longer show a "\\?\" in front (the Hardwave drums folder and every sample taken from it); songs saved with those paths open as before and show them without it
- fix: FL Studio's 909 Clap and 909 Snare (WAV files with Ogg Vorbis audio inside) now load and play instead of being refused
- fix: the Edit menu now has Auto-crossfade overlaps, which the help already pointed to: every overlap between two audio clips of one channel on a playlist track gets a crossfade, as one undo step
- fix: a sampler track built quickly (track, Sampler, sample, notes) no longer stays silent until the song is saved and reopened.
- fix: a song's plug-in windows open again after you open the song a second time in the same session (Hard-Shape said it had no window).
- fix: exports use a plug-in's saved settings when the playing plug-in hands over nothing, so a sampler's sample is never left out of a render.
- fix: cancelling an export closes the export window quietly instead of showing "Export failed: Export cancelled".
- fix: the tempo lane shows a starter song's own tempo (150 BPM for the hardstyle starter) instead of 140.
- fix: the browser's Plug-ins tab no longer says "Click Scan to find VST3 & CLAP plugins" while your plug-ins are still loading.

### Improvements
- fix(export): a plug-in that hands over nothing keeps its saved settings
- fix(plugins): a reopened song's plug-in windows open again
- fix(browser): "Click Scan" shows only when no plug-ins are known
- fix(playlist): the tempo lane shows a starter song's own tempo
- fix(export): cancelling an export closes the dialog without an error box
- fix(engine): a sample loaded right after adding a sampler is never lost
- fix(files): Windows paths no longer show the \\?\ prefix
- fix(playlist): renaming a channel renames the playlist row made for it
- fix(playlist): Edit > Auto-crossfade overlaps exists and crossfades as one undo step
- fix(browser): the Plug-ins tab lists the known plug-ins without a Scan
- fix(audio): FL's 909 Clap and Snare load: a WAV holding Ogg Vorbis is read
