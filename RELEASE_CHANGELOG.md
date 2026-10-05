### Improvements
- feat(daw): the audio work can be shared across cores
- The audio work can be shared across cores. A song where one track costs more than a block's budget could not play however many cores the machine had, because the graph ran on the audio thread alone. Tracks that do not feed each other run at the same time now. Settings > Audio picks how many threads; it starts at one until you have tried it on your machine, and the render is the same either way.
