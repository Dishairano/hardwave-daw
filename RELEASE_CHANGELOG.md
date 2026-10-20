### Bug fixes
- fix: when WebView2 fails to open the main window, the DAW starts again (3 tries) and then shows why, instead of running unseen
- fix: when WebView2 never answers, the DAW shows the same message after 6 minutes and closes
- fix: certificate checks and root certificates updated for two new security advisories (RUSTSEC-2026-0381, RUSTSEC-2026-0382)

### Improvements
- fix(deps): patched certificate checks and root certificates
- fix(window): say so when WebView2 never answers for the main window
- fix(window): start again when WebView2 does not open the main window
