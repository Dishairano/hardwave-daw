### Improvements
- The audio work can be shared across cores. A song where one track costs more than a block's budget could not play however many cores the machine had, because the graph ran on the audio thread alone. Tracks that do not feed each other run at the same time now. Settings > Audio picks how many threads; it starts at one until you have tried it on your machine, and the render is the same either way.
- Ableton Link. Settings > MIDI joins a session and says how many other apps are in it; the tempo and the start and stop are shared both ways with everything on the network that speaks Link.
- internal: nodes are grouped into levels so two nodes that run together can never have an edge between them, and a pool of threads stays alive between blocks; the engine test renders the same song on one core and on four and compares every sample.
- internal: builds on the Windows gate machine run between 03:00 and 08:00 only, guarded in release.sh and in the workflow.
