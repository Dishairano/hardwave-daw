### Improvements
- perf(daw): denormal numbers no longer spike the CPU at the end of a tail
- fix(daw): stop sending panel-window diagnostics to our server
- The DAW no longer sends details about your machine to us when you pop a panel into its own window: that debug reporting is gone, with the collector and its logs.
- Long reverb and delay tails no longer make the CPU meter climb for no reason, which could cause dropouts near the end of a song.
