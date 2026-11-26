### Bug fixes
- fix: on a very busy computer the DAW no longer closes right after starting when Windows is slow to open its window (WebView2 error 0x800705B4). It says that it is trying again, waits a little longer each time, and opens on the next try; after three tries it says why it cannot open instead of closing without a word
