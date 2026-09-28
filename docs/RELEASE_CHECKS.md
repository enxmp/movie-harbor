# Initial public release checks

Reviewed on 2026-09-28. This records a targeted release review, not a security certification.

- A clean source checkout was assembled without personal conversion logs, partial movies, previous binary archives, connection details, or SSH files. Git commit attribution uses the project's GitHub noreply address.
- Gitleaks scanned the staged source and found no secrets. An additional check covered the developer's known private hostnames, usernames, share addresses, and local paths.
- `npm audit` reported zero known advisories for the JavaScript lockfile.
- `cargo audit` reported zero vulnerability entries. The all-platform lockfile includes the informational `proc-macro-error` unmaintained warning (RUSTSEC-2024-0370) and the `glib` unsoundness warning (RUSTSEC-2024-0429). Both packages were absent from the Windows x64 and macOS arm64 dependency graphs; this release does not ship a Linux build.
- Windows and macOS Rust tests cover authentication rejection, browser-origin rejection, folder restrictions, server shutdown, exclusive file moves, encoding-plan checks, and queue continuation. The macOS tests additionally check symlink escapes.
- The first macOS DMG had an incomplete linker-only signature. The replacement app is ad hoc signed as a complete bundle. `codesign --verify --deep --strict` passes both before packaging and after mounting the verified DMG. No Apple Developer ID certificate is available, so Gatekeeper can still require manual approval.
- Real local FFmpeg tests exercised a failed job followed by a successful job and E-AC-3 conversion. Browser tests exercised the conversion UI and Windows/Mac remote settings with mocked native calls.
- The real conversion screenshot contains no private paths, SSH information, or access tokens.

The release requires an independently installed FFmpeg build. This review does not audit that external binary, GPU drivers, WebView2, WebKit, or the operating system. Dependency advisories can change after this date.
