## First public beta

Movie Harbor is a focused movie-library converter for Windows and Apple Silicon: hardware HEVC, explicit audio-track choices, optional 1080p downscaling, and a compact queue.

- NVIDIA NVENC and Apple VideoToolbox; video passthrough and original frame timing.
- AAC, E-AC-3, or audio passthrough, with recommended outputs and retained track names.
- One or two simultaneous jobs, supported NVIDIA split-frame encoding, and per-movie progress underlines.
- Queue continues after an item fails; bounded repair for small audio timestamp regressions.
- Subtitles, chapters, font attachments, deinterlacing, output validation, backup controls, and exact-job cleanup.
- Configurable SSH remote conversion with opt-in hosting, per-session authentication, and folder restrictions.

**Downloads:** Windows x64 `.exe` and Apple Silicon `.dmg`. FFmpeg/ffprobe are required separately. No ZIP is needed for Windows. The macOS app bundle is sealed with an ad hoc signature, but the release has no Developer ID signature or Apple notarization; Gatekeeper approval may be needed. SHA-256 hashes and third-party notices are attached.

**Before converting:** Replace original defaults on and original-backup retention defaults off. Disable replacement or enable backups when you want to retain the source. HDR/Dolby Vision needs video passthrough. The queue is session-only; output checks use metadata and short decode samples.

See the README for setup, features, limitations, and remote-control instructions.
