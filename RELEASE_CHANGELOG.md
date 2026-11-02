### Bug fixes
- fix: 24-bit WAV samples, the depth most sample packs use, came in silent in any song (drag-in, sampler, FL import); they now play, as do 8-bit and 64-bit float WAVs
- fix: FL samples fitted to the tempo with FL's TIME knob play at the length FL gives them and start where FL starts them (they played at about half speed, and trimmed loops started in the wrong place)
- fix: Import FL Studio project now also opens FL's zipped loop package (.zip): it unpacks into a folder next to the zip, refuses unsafe archives, and the song plays the samples in that folder

### Improvements
- fix(audio): 24-bit WAV samples play instead of coming in silent
- fix(fl import): a project shared as FL's zipped loop package opens with its samples
