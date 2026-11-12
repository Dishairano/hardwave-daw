### Bug fixes
- fix: the first export after opening an FL song no longer freezes the window; while stretched clips are prepared the export window says "preparing stretched audio, n of N" and Cancel works
- fix: an export only prepares the stretched clips in the part being exported, so exporting part of a song starts much sooner
- fix: stretched and pitched clips are prepared over twice as fast, before playback and before an export, on several cores at once
- fix: big songs play without dropouts on slower computers: tracks with nothing playing at that moment no longer use the processor
- fix: the built-in sampler no longer copies its sample all the time while the song plays, a cause of crackles in songs with sampler channels
- fix: measuring each track's load in the mixer no longer slows playback down; on some Windows machines it took a fifth of the processing time
- fix: the built-in EQ uses less processor time and sounds the same
- fix: instrument tracks with a plug-in synth no longer play a hidden built-in synth under every note
- fix: pitched audio clips play at the right level (a loop raised by 13 semitones came out 1.6 dB too loud) and stretched clips sound slightly different, closer to the original

### Improvements
- test(engine): the graph swap test checks what the audio thread built, not the graph number
- fix(engine): an instrument track with a plug-in synth no longer runs a hidden built-in synth for every note
- fix(eq): the built-in EQ does the same work in less time
- fix(engine): a track with nothing playing in a block no longer runs its whole channel over silence
- fix(midi): reading the keyboard on each block no longer reads the clock when nothing was played
- fix(engine): the per-track load read-out no longer reads the clock twice per track on every block
- fix(sampler): the built-in sampler no longer copies its whole sample on every block
- test(engine): measure what playing an FL song costs the audio thread, block by block
- fix(export): stretched clips are prepared on several cores before an export
-2000 rad. The notes in a stretched chord come out at the same level
- fix(export): the window keeps working while an export prepares stretched clips
