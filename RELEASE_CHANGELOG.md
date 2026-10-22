### Bug fixes
- fix: turning a knob in a plug-in's own window is recorded as automation again
- fix: picking a VST3 factory preset switches the plug-in to it; for most plug-ins it changed nothing
- fix: the chosen audio driver (ASIO, WASAPI) is kept between launches
- fix: the extra engine threads flush denormals too, so quiet reverb tails no longer spike the CPU there

### Improvements
- fix(engine): the worker threads and offline renders flush denormals too
- fix(audio): the chosen audio driver is kept between launches
- fix(plugins): picking a VST3 factory preset switches the plug-in to it
- fix(automation): knob moves in a plug-in's own window are recorded
