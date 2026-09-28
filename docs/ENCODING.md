# Encoding choices

## Video

NVIDIA output uses `hevc_nvenc`, Main10, P7/HQ, VBR constant quality, and CQ 24 by default. Apple output uses hardware-only `hevc_videotoolbox`, Main10; the default UI quality 24 maps to VideoToolbox quality 52. These encoder scales are different.

Video passthrough copies the compressed stream. Encoding uses `fps_mode passthrough` and approximately two-second forced keyframes. No interpolation or explicit frame-rate conversion is requested. Metadata frame rates are checked after conversion.

Downscaling fits inside 1920 × 1080 with even dimensions and preserved aspect ratio. It never upscales. Flagged interlaced sources use BWDIF with one output frame per input frame, clear field flags, and require progressive output metadata. Source flags may be wrong; inverse telecine is not supported.

HDR and Dolby Vision sources must use video passthrough. This release does not implement HDR-to-SDR conversion or HDR/Dolby Vision transcoding.

## Audio

The recommendation policy preserves AAC, AC-3, E-AC-3, Opus, MP3, Vorbis, supported lossless codecs, PCM, recognized object-audio tracks, unknown layouts, and tracks above 5.1. Other mono/stereo tracks become AAC; supported surround becomes E-AC-3. Metadata can be incomplete, so review important Atmos/DTS:X tracks yourself.

| Output | Bitrate |
| --- | --- |
| AAC mono/stereo | 256 kb/s |
| AAC up to 5.1 | 512 kb/s |
| AAC up to 7.1 | 768 kb/s |
| E-AC-3 mono/stereo | 256 kb/s at 48 kHz |
| E-AC-3 surround up to 5.1 | 640 kb/s at 48 kHz |

Unsupported E-AC-3 layouts are rejected instead of silently downmixed. Channel count, language, and track names are checked. Lossy conversion does not preserve lossless or object-audio extensions. Player support for multichannel AAC varies.

Timestamp repair changes packet timestamps, not audio payloads on passthrough tracks. It moves a packet forward by at most 250 ms and leaves at least one millisecond between packet DTS values for MKV precision. Larger regressions still fail under strict FFmpeg error handling. This is a bounded repair for small source discontinuities, not reconstruction of missing or severely damaged audio.

## Queue and files

One or two workers process independent movies. Failure does not stop later items; explicit cancellation stops active workers and leaves pending jobs ready. Recognized dual-NVENC desktop GPUs default to two jobs and, when supported by FFmpeg, split-frame encoding. Both modes share hardware and memory; neither guarantees twice the speed.

Each job writes a temporary file beside its destination. After FFmpeg succeeds, the app checks dimensions, frame-rate metadata, duration, track/chapter counts, audio codec/language/channel count/name, and short decode samples at the start, middle, and end. This is not a full integrity scan.

Replacement first moves the original to an exclusive rollback name. Publication uses a hard link or, on supported macOS SMB shares, an exclusive rename. Existing destinations are never deliberately overwritten by publication or rollback. If the filesystem cannot provide the required operation, the job fails and retains recovery files. Backups are removed only after successful replacement and only when backup retention is off. The app never sweeps a media folder for old files.
