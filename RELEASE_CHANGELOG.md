### Bug fixes
- fix: on Windows the DAW now runs its audio at real-time priority (the way other DAWs do), so heavy songs stop crackling and dropping out when the computer is busy, and the CPU meter reads far lower. This covers WASAPI and ASIO; an ASIO driver that already sets its own real-time priority (FL Studio ASIO does) keeps it.
- fix: when a launcher or Task Scheduler starts the DAW at low priority, it now raises itself to normal priority, so songs no longer keep dropping out
- fix: security updates: the date library chrono, which is no longer maintained, is replaced, and the url and cookie libraries are updated for two advisories published this week (RUSTSEC-2026-0592 and RUSTSEC-2026-0594)

### Improvements
- fix(audio): each new audio stream writes its thread priority to the log
- fix(audio): a stopped input no longer reports its old thread priority
- fix(audio): an ASIO driver's own real-time priority is no longer lowered
- fix(app): the DAW no longer stays in the below-normal priority class when a launcher starts it there
- fix(engine): the helper threads get the audio thread's priority, and the app reports it
- fix(audio): run the audio thread at real-time priority on Windows
- fix(app): the session log says when the system time zone cannot be read
