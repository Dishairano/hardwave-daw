### New features
- feat: a pattern clip in the playlist shows its notes, as in FL Studio, so a bass line and a lead read apart at a glance
- feat: a sample library (Documents\Hardwave\Library, in the browser's Places): Add pack unpacks a pack's .zip in one step, and a pack is removed again to the Recycle Bin. Only samples, MIDI, presets, images and text are unpacked, never a program, and a zip that unpacks to more than it claims is stopped
- feat: plug-in windows resize: drag the window and the plug-in follows (or keeps its size when it cannot), and a plug-in that asks for another size gets it

### Bug fixes
- fix: a plug-in window follows the screen's scale, so at 150% it draws sharp and at full size instead of small in a corner

### Improvements
- fix(browser): the pack button fits beside Add folder
- fix(build): the Windows build picks flate2's backend for the zip reader
- feat(playlist): a pattern clip shows its notes, as in FL Studio
- feat(browser): a sample library, with packs installed from their zip and removed to the Recycle Bin
- feat(plugins): plug-in windows resize, follow the screen's scale, and grow when a plug-in asks
