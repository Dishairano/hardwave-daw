### Bug fixes
- fix: editing a big song while it plays no longer makes the audio drop out: the DAW rebuilds the song in the background and switches over between two blocks
- fix: notes, kick tails, filters and delays keep sounding when you edit while the song plays; before, every edit cut whatever was ringing
- fix: a note held by an instrument plug-in is released when you delete it while it plays, instead of hanging

### Improvements
- Each audio clip looked its sample up in the pool up to four times
- Compensation looked up the slowest branch into an edge's destination
- Walking a chain of buses to find loops searched the whole track list
- fix(engine): an edit no longer builds the audio graph on the audio thread
- fix(merge): the bounce test's automation lane says it has no row yet
