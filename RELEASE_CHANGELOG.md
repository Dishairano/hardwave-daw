### Bug fixes
- fix: if a bounce, freeze or consolidate cannot finish (the track was removed while it rendered, or the rendered file cannot be read), your song and your undo and redo steps stay exactly as they were: no empty undo step, no lost redo, no vanished clips and no empty "(bounced)" track. Each of them was already one undo step when it works
- fix: unfreezing a track that is not frozen, or dropping a sample that cannot be loaded, no longer adds an empty undo step or throws away your redo steps
