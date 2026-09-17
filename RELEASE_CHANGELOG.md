### New features
- feat: a song can change time signature part-way through. Set it on a tempo map entry and the grid, the bar numbers, the click and your plug-ins all follow it from that point on
- feat: the tempo map dialog now has a time signature column, so a change can actually be made
- feat: the paint tool now paints. Drag across the playlist and the clip you picked lands in every slot you cross, across bars and down tracks

### Bug fixes
- fix: the time signature is saved with the project. It used to be lost on save and every project came back in 4/4
- fix: bar numbers keep counting through the whole song. Adding a tempo change used to restart them, so one song showed bar 1 more than once
- fix: signatures like 7/8 are counted in eighth notes. Bars were drawn twice as wide as they sounded
- fix: adding a tempo change in the middle of a bar no longer shifts the bar lines
- fix: the bar counter in the toolbar now agrees with the bar numbers over the playlist
- fix: with the playlist scrolled down, clicking to place a clip put it on the track under your cursor instead of one several rows higher

### Improvements
- improve: a time signature that cannot be counted, such as 4/3, is refused with a message instead of being stored
- improve: bar numbers in the ruler are dropped rather than drawn over each other when you zoom out
- improve: painting skips slots that already hold a clip, so a copy is never dropped on top of an existing one
- improve: the hint bar now says what the paint tool needs when nothing is picked yet, instead of the tool doing nothing without explanation
