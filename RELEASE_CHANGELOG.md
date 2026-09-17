### Bug fixes
- fix: previewing a sample in the browser now plays through the audio device the DAW is using, so you can hear it with ASIO and exclusive-mode drivers instead of getting silence or sound from the wrong speakers
- fix: the preview volume slider in the browser now changes the sound you actually hear, including while a sample is still playing
- fix: a sample recorded at a different sample rate now auditions at the right pitch instead of slightly sharp or flat
- fix: a mono sample now auditions through both speakers instead of only the left one

### Improvements
- improve: clicking a second sample replaces the first instead of the two playing on top of each other, and an audition stops by itself at the end of the file
- improve: a sample you audition more than once is only read from disk once, so clicking back and forth through a folder stays instant
