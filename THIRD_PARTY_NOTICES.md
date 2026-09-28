# Third-party software

Movie Harbor's own source is MIT licensed. Third-party packages retain their respective licenses; the lockfiles pin the versions used in a build.

- Tauri and its official plugins: MIT or Apache-2.0.
- React and React DOM: MIT.
- Tailwind CSS, Vite, and their official build integrations: MIT.
- Lucide icons: ISC.
- Rust ecosystem dependencies: see the package manifests and license files in their published crates, including serde, serde_json, getrandom, libc, and windows-sys.
- Windows WebView2 and macOS WebKit are platform runtimes with their own terms.

FFmpeg and ffprobe are external executables. They are not linked into Movie Harbor or bundled in its release downloads. Their license depends on how they were built; consult [FFmpeg's license information](https://ffmpeg.org/legal.html) and your distributor's notices/source offer before redistributing an FFmpeg build.

The application icon was generated for this project. The screenshot shows the app's interface and filenames during a real conversion; it does not include movie footage or distribute those movies.
