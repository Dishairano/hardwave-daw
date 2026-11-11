### New features
- feat: FL Studio projects open the way FL shows them: every channel in the channel rack in FL's groups, every clip on its FL playlist row, and the mixer's inserts numbered and named as in FL (no more 500 placeholder tracks)
- feat: a channel's volume and pan from FL are on the channel, so FL's channel volume automation works
- feat: automation clips from FL sit on their own automation channels, on the row FL had them
- feat: the channel rack filters by the song's groups (All, then each group with its count), as in FL
- feat: the playlist shows rows like FL Studio: each row has a name, a colour and its own Mute and Solo; double-click a name or press F2 to rename; right-click a row for colour, mute, solo, insert or delete
- feat: dragging a clip up or down moves it to another row without changing how it sounds; any row can hold clips from any channel
- feat: muting a row silences the clips on it; soloing a row plays only the soloed rows
- feat: a sample dropped on the playlist gets its own channel, playing into the next free mixer insert, and lands on the row you dropped it on
- feat: automation clips sit on the playlist like other clips: move them to any row, resize them, double-click to add a point, right-click a point to remove it
- feat: hovering a clip shows which insert it plays through
- feat: the mixer shows your inserts by number and name, like FL; empty numbers are drawn up to 125, and a channel without its own insert no longer takes a strip
- feat: rename an insert by double-clicking its name, pressing F2, or right-clicking it (Rename, Colour)
- feat: put an effect on an empty insert, or route something to it, and it becomes an insert, as one undo step
- feat: the channel rack's insert box shows the insert each channel plays into; click it to pick from the list, or drag or scroll on it to step through numbers
- feat: the Output and Send lists in the mixer show inserts by number, including the empty ones
- feat: in new songs and FL imports, mixer inserts sit at 0 dB with their pan centred, as in FL, so routing a channel into an insert no longer makes it 3 dB quieter; songs you made before keep sounding exactly as they did
- feat: naming an insert that already exists renames it, as one undo step
- feat: the channel rack's insert box takes any insert number you type up to 2000, and says so when what you typed is not one

### Bug fixes
- fix: pasted clips stay on the channel they came from
- fix: a new, imported or recorded clip stays on its playlist row when you move its channel's other clips, and moves with its row when rows are inserted or deleted
- fix: the sample window's Track box shows the same insert number as the mixer and the channel rack
- fix: a track you add shows in the mixer and the rack right away, with the next free number
- fix: Hardwave Reverb now makes reverb: a real tail whose length follows Decay, Size and Damping that work, and a Mix that blends from dry to reverb only at a steady level
- fix: Conv Reverb no longer makes a track 20 dB louder; at its default settings and at 100% wet it plays about as loud as the dry sound
- fix: freezing, bouncing or consolidating a part that runs above 0 dBFS no longer flattens it; the frozen track sounds exactly as it did live
- fix: a clip placed on top of a longer one on the same track no longer fades the longer one out; both play at full level, so consolidating such a range no longer changes the sound
- fix: synths on a MIDI track (3xOSC, Wavetable, FM and the built-in synth) now hold their notes for the full length during playback; before, a note could fade out after a fraction of a second on many Windows sound cards, even though the export played it in full; kick notes also land exactly on the beat again
- fix: a plug-in setting changed while the song is stopped now takes effect straight away and shows its new value, instead of only once you press play
- fix: loading a preset right after turning a knob no longer brings the old knob value back
- fix: Stutter's Sync setting is now saved with the song and stays as you set it

### Improvements
- test(engine): the graph swap test checks what the audio thread built, not the graph number
- change: songs are saved in format 4; older DAW versions say the song was made by a newer version
