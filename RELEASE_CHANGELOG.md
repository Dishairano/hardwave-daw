### Improvements
- test(engine): a seek sends the value a controller was left at
- feat(daw): the distortion stops folding harmonics back down
- fix(daw): starting in the middle keeps the controller values the song had
- Starting playback in the middle of a song now sends the controller values the song had written by that point, so a filter opened by an earlier mod-wheel move is open.
- The distortion runs its curve at twice the sample rate by default, which stops the harshness that comes from harmonics folding back down. There is an Oversample control for off, 2x or 4x.
