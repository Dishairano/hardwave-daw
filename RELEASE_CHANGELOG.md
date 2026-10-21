### Bug fixes
- fix: the room connection now checks the service's certificate against the patched root list (webpki-roots 1.0.10); the outdated copy flagged by RUSTSEC-2026-0381 is gone from the build

### Improvements
- fix(build): ask windows-sys for SystemInformation ourselves
- fix(collab): room TLS from webpki-roots 1, the certificate exception goes
