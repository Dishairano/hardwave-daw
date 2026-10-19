### New features
- feat: Hardwave DAW has its own removal window in Windows Apps: it says what stays and what goes, and keeps your projects, presets and recordings
- feat: Normalize, Remove DC offset, Reverse, Reverse polarity and Swap stereo work on sampler channels
- feat: Load regions and Load slice markers show a WAV file's markers in the sample window
- feat: FL audio-clip channels keep normalize, remove DC, fade stereo, trim and declicking
- feat: FL sampler channels keep their volume envelope, loop, mono, porta, cut itself, declicking, trim and fade stereo

### Bug fixes
- fix: CLAP plug-ins load in the DAW (every CLAP plug-in was refused before)
- fix: a plug-in that will not start is no longer reported as a crash
- fix: the release no longer carries the developers' test panel and test files
- fix: samples at a different sample rate than your audio device play on time and no longer lose their last milliseconds
- fix: playing a new or mostly empty song no longer stutters on slower PCs; empty mixer tracks use almost no CPU
- fix: a sampler channel's window shows its real settings when the audio device is not running
- fix: the DAW starts on a Windows PC that does not have the Visual C++ Redistributable
- fix: the sample window's Resample and Keep on disk options say what the DAW does
- fix: loading a sample into a sampler channel keeps its envelope and other settings, and takes the file's loop
- fix: the sample window's Track box shows the mixer insert, not 501

