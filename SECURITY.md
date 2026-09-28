# Security

## Reporting

Use [GitHub private vulnerability reporting](https://github.com/enxmp/movie-harbor/security/advisories/new) for security issues. Do not publish access tokens, SSH keys, private paths, or media files in a public issue. Ordinary bugs can go to GitHub Issues.

## Security model

- The app processes local or mounted-share files using your installed FFmpeg/ffprobe. Media subprocesses receive argument arrays, not interpolated shell commands. Only `file` and `pipe` input protocols are enabled.
- No telemetry, cloud upload, advertising, automatic updater, or bundled account credentials are included.
- The webview loads bundled UI assets with a restrictive Content Security Policy. No remote webview origins receive native capabilities.
- Executable paths selected in Settings are trusted programs and run with your user privileges. Choose only trusted FFmpeg builds, and keep them updated: malformed media is processed by FFmpeg and its codec libraries.
- Remote hosting is disabled by default, binds to loopback only, and requires a fresh OS-random session token. Browser-origin requests, oversized requests, duplicate headers, and chunked requests are rejected. Reads have a time limit; requests are processed without spawning an unbounded thread per connection.
- SSH uses the user's selected key and strict known-host verification. No hostname, username, private network address, key filename, or share path is built in. Tokens are kept in process memory and are not saved to disk or command-line arguments.
- Remote file paths must resolve inside the folder shared on the Mac. Remote clients cannot select executables or invoke arbitrary shell commands through the protocol. Access to a token and working SSH connection grants conversion, replacement, and cancellation authority within that folder.
- File publication and rollback use operations that fail when the destination already exists. Successful cleanup targets exact files created by that job; failures preserve partial output and logs.

## Release review

The first public release was prepared separately from the developer's personal installation. Personal connection settings, conversion logs, temporary media, old binary archives, and SSH-related files are excluded from the repository. The real conversion screenshot was reviewed for private paths and connection details.

Review includes source inspection, secret scanning, dependency advisory checks, builds, queue-continuation tests, authentication/path-scope tests, and file-publication collision tests. This is a targeted review, not an independent penetration test or a guarantee that the application or its dependencies contain no vulnerabilities.

The app runs as the current user; it is not a sandbox for hostile media or local malware. Replacement with backup retention disabled intentionally deletes the original after successful checks. The checks are samples and metadata checks, not a complete frame-by-frame quality assessment.

Initial binaries are unsigned and not Apple-notarized. Download from this repository's release page and compare the published SHA-256 hashes. Only the latest release is maintained.
