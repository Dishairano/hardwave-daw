### Bug fixes
- fix: changing the audio settings twice in quick succession (or once while a song was opening) could load every plug-in twice, so the song played at twice the level and a plug-in in its own process ran two copies. A plug-in loaded again now takes the place of the one already there
- fix: when the sound card was unplugged or taken by another program, restarting the audio waited half a second for plug-in settings it could not get, with the app stalled; it now goes straight on with the settings as last saved
- fix: a plug-in taken off a track, or replaced when the song's plug-ins were loaded again, was only freed the next time a plug-in was added or removed by hand; until then it kept its memory, and a plug-in in its own process kept running idle. It is now freed within half a second

### Improvements
- fix(plugins): a plug-in the audio thread lets go of is freed within half a second
- Two audio restarts more than a tick apart each started loading the
- After the audio stream failed (the card unplugged or taken by another
