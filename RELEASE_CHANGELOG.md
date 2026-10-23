### New features
- feat: clips snap to other clips' edges and to markers within a few pixels (Alt still turns snapping off)
- feat: Edit > Delete and close the gap (Ctrl+Del) deletes the selected clips and moves what follows back

### Bug fixes
- fix: a MIDI keyboard plays one track: the armed ones, or else the MIDI track picked last; it played every MIDI track at once
- fix: Write, Latch and Touch each do what they say; Latch and Write keep going after the release and hold the last value until the song stops
- fix: track folders are saved with the song; they lived on the machine, so every song showed the last song's folders

### Improvements
- feat(playlist): delete and close the gap (ripple delete)
- feat(playlist): clips snap to other clips' edges and to markers
- fix(playlist): track folders are saved with the song, not on the machine
- fix(automation): Write, Latch and Touch each do what they say
- fix(midi): a keyboard plays one MIDI track, not all of them
