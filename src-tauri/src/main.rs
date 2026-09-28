#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
mod remote;
use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Default, Serialize)]
struct Progress {
    running: bool,
    phase: String,
    seconds: f64,
    speed: String,
    message: String,
    output: String,
}
#[derive(Default)]
struct Shared {
    progress: Mutex<Progress>,
    cancel: AtomicBool,
}
#[derive(Default)]
struct Queue {
    entries: Mutex<Vec<Entry>>,
    active: AtomicBool,
    halted: AtomicBool,
}
struct Entry {
    id: String,
    job: Job,
    state: Arc<Shared>,
}
#[derive(Deserialize)]
struct Submission {
    id: String,
    job: Job,
}
#[derive(Clone, Deserialize, Serialize)]
struct Audio {
    index: u64,
    mode: String,
}
#[derive(Clone, Deserialize, Serialize)]
struct Job {
    #[serde(default = "default_keep_backup")]
    deinterlace: bool,
    #[serde(default)]
    keep_backup: bool,
    #[serde(default)]
    split_encode: bool,
    #[serde(default)]
    repair_audio_timestamps: bool,
    source: String,
    folder: String,
    replace: bool,
    video: String,
    downscale: bool,
    quality: u8,
    audio: Vec<Audio>,
    subtitles: Vec<u64>,
    ffmpeg: String,
    ffprobe: String,
}

fn default_keep_backup() -> bool {
    true
}
fn command(exe: &str) -> Command {
    let mut c = Command::new(exe);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x08000000);
    }
    c
}
fn executable(name: &str) -> String {
    if let Ok(p) = std::env::current_exe() {
        if let Some(dir) = p.parent() {
            let candidate = dir.join(if cfg!(windows) {
                format!("{name}.exe")
            } else {
                name.into()
            });
            if candidate.is_file() {
                return candidate.to_string_lossy().into_owned();
            }
        }
    }
    if cfg!(target_os = "macos") {
        for dir in ["/opt/homebrew/bin", "/usr/local/bin"] {
            let p = Path::new(dir).join(name);
            if p.is_file() {
                return p.to_string_lossy().into_owned();
            }
        }
    }
    name.into()
}
fn probe(path: &str, tool: &str) -> Result<Value, String> {
    let path = fs::canonicalize(path).map_err(|e| format!("Cannot open local media file: {e}"))?;
    if !path.is_file() {
        return Err("Select a local media file".into());
    }
    let out = command(tool)
        .args([
            "-v",
            "error",
            "-protocol_whitelist",
            "file,pipe",
            "-show_streams",
            "-show_format",
            "-show_chapters",
            "-of",
            "json",
            "-i",
            &path.to_string_lossy(),
        ])
        .output()
        .map_err(|e| format!("Cannot launch ffprobe: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr)
            .chars()
            .take(1200)
            .collect());
    }
    serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())
}
fn streams(p: &Value) -> Result<&Vec<Value>, String> {
    p["streams"].as_array().ok_or("No media streams".into())
}
fn video(p: &Value) -> Result<&Value, String> {
    streams(p)?
        .iter()
        .find(|s| {
            s["codec_type"] == "video"
                && s["disposition"]["attached_pic"] != 1
                && s["disposition"]["timed_thumbnails"] != 1
        })
        .ok_or("No main video stream".into())
}
fn hdr(v: &Value) -> bool {
    matches!(
        v["color_transfer"].as_str(),
        Some("smpte2084" | "arib-std-b67")
    ) || v["side_data_list"].to_string().contains("DOVI")
}
fn interlaced(v: &Value) -> bool {
    matches!(v["field_order"].as_str(), Some("tt" | "bb" | "tb" | "bt"))
}
fn duration(p: &Value) -> f64 {
    p["format"]["duration"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0)
}
fn dimensions(v: &Value, downscale: bool) -> (u64, u64) {
    let w = v["width"].as_u64().unwrap_or(0);
    let h = v["height"].as_u64().unwrap_or(0);
    if !downscale || (w <= 1920 && h <= 1080) {
        return (w, h);
    }
    let scale = (1920.0 / w as f64).min(1080.0 / h as f64).min(1.0);
    (
        ((w as f64 * scale / 2.0).floor() as u64) * 2,
        ((h as f64 * scale / 2.0).floor() as u64) * 2,
    )
}
fn audio_timestamp_repair() -> &'static str {
    // Preserve payload and PTS/DTS offset. Leave larger regressions for -xerror to reject.
    "setts=pts='if(eq(N,0),PTS,if(lte(PREV_OUTDTS+max(1,ceil(0.001/TB))-DTS,0.25/TB),PTS+max(0,PREV_OUTDTS+max(1,ceil(0.001/TB))-DTS),PTS))':dts='if(eq(N,0),DTS,if(lte(PREV_OUTDTS+max(1,ceil(0.001/TB))-DTS,0.25/TB),max(DTS,PREV_OUTDTS+max(1,ceil(0.001/TB))),DTS))'"
}
fn plan(j: &Job, p: &Value) -> Result<Vec<String>, String> {
    let v = video(p)?;
    let all = streams(p)?;
    if !["copy", "nvenc", "apple"].contains(&j.video.as_str()) {
        return Err("Invalid video mode".into());
    }
    if j.split_encode && j.video != "nvenc" {
        return Err("Split-frame encoding requires NVIDIA HEVC".into());
    }
    if j.video == "copy" && j.downscale {
        return Err("Downscaling requires video encoding".into());
    }
    if j.video != "copy" && hdr(v) {
        return Err("HDR/Dolby Vision: select video passthrough to preserve dynamic-range metadata. HDR transcoding is not supported in this version.".into());
    }
    if j.video != "copy" && interlaced(v) && !j.deinterlace {
        return Err("Source is flagged as interlaced. Enable automatic deinterlacing under Conversion options, or choose video passthrough.".into());
    }
    if !(16..=32).contains(&j.quality) {
        return Err("Quality must be 16–32".into());
    }
    let mut a: Vec<String> = [
        "-hide_banner",
        "-nostdin",
        "-n",
        "-xerror",
        "-protocol_whitelist",
        "file,pipe",
        "-i",
        &j.source,
        "-map",
        &format!("0:{}", v["index"]),
        "-map_metadata",
        "0",
        "-map_chapters",
        "0",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let mut seen = std::collections::HashSet::new();
    for (n, t) in j.audio.iter().enumerate() {
        let s = all
            .iter()
            .find(|s| s["index"] == t.index && s["codec_type"] == "audio")
            .ok_or("Invalid audio selection")?;
        if !seen.insert(t.index) || !["copy", "aac", "eac3"].contains(&t.mode.as_str()) {
            return Err("Invalid or duplicate audio track".into());
        }
        a.extend([
            "-map".into(),
            format!("0:{}", t.index),
            format!("-c:a:{n}"),
            t.mode.clone(),
        ]);
        if j.repair_audio_timestamps {
            a.extend([format!("-bsf:a:{n}"), audio_timestamp_repair().into()]);
        }
        if t.mode == "eac3" {
            let channels = s["channels"].as_u64().unwrap_or(0);
            let layout = s["channel_layout"].as_str().unwrap_or("");
            if channels == 0
                || channels > 6
                || ![
                    "mono",
                    "stereo",
                    "3.0(back)",
                    "3.0",
                    "quad(side)",
                    "quad",
                    "4.0",
                    "5.0(side)",
                    "5.0",
                    "2 channels (FC+LFE)",
                    "2.1",
                    "4 channels (FL+FR+LFE+BC)",
                    "3.1",
                    "4.1",
                    "5.1(side)",
                    "5.1",
                ]
                .contains(&layout)
            {
                return Err("E-AC-3 requires a supported layout up to 5.1. Use passthrough or AAC; no automatic downmix is allowed.".into());
            }
            a.extend([
                format!("-b:a:{n}"),
                if channels <= 2 { "256k" } else { "640k" }.into(),
                format!("-ar:a:{n}"),
                "48000".into(),
                format!("-channel_layout:a:{n}"),
                layout.into(),
            ]);
        }
        if t.mode == "aac" {
            let channels = s["channels"].as_u64().unwrap_or(2);
            if channels > 8 {
                return Err(
                    "AAC conversion supports at most 8 channels; use passthrough for this track"
                        .into(),
                );
            }
            a.extend([
                format!("-b:a:{n}"),
                format!(
                    "{}k",
                    if channels <= 2 {
                        256
                    } else if channels <= 6 {
                        512
                    } else {
                        768
                    }
                ),
            ]);
        }
        if let Some(title) = s["tags"]["title"].as_str().or(s["tags"]["name"].as_str()) {
            a.extend([format!("-metadata:s:a:{n}"), format!("title={title}")]);
        }
    }
    for (n, id) in j.subtitles.iter().enumerate() {
        let s = all
            .iter()
            .find(|s| s["index"] == *id && s["codec_type"] == "subtitle")
            .ok_or("Invalid subtitle selection")?;
        if !seen.insert(*id) {
            return Err("Duplicate subtitle".into());
        }
        a.extend([
            "-map".into(),
            format!("0:{id}"),
            format!("-c:s:{n}"),
            if s["codec_name"] == "mov_text" {
                "srt"
            } else {
                "copy"
            }
            .into(),
        ]);
    }
    // Only fonts, never cover-art attachments.
    for s in all {
        if s["codec_type"] == "attachment" {
            let name = s["tags"]["filename"].as_str().unwrap_or("").to_lowercase();
            if [".ttf", ".otf", ".ttc"]
                .iter()
                .any(|ext| name.ends_with(ext))
                && !j.subtitles.is_empty()
            {
                a.extend([
                    "-map".into(),
                    format!("0:{}", s["index"]),
                    "-c:t".into(),
                    "copy".into(),
                ]);
            }
        }
    }
    if j.video == "copy" {
        a.extend(["-c:v".into(), "copy".into()]);
    } else {
        let (w, h) = dimensions(v, j.downscale);
        if w == 0 || h == 0 {
            return Err("Invalid source dimensions".into());
        }
        a.extend([
            "-vf".into(),
            format!(
                "{}scale={w}:{h}:flags=lanczos,format=p010le",
                if j.deinterlace && interlaced(v) {
                    "bwdif=mode=send_frame:parity=auto:deint=interlaced,setfield=prog,"
                } else {
                    ""
                }
            ),
            "-fps_mode".into(),
            "passthrough".into(),
            "-force_key_frames".into(),
            "expr:gte(t,n_forced*2)".into(),
        ]);
        if j.deinterlace && interlaced(v) {
            a.extend(["-field_order".into(), "progressive".into()]);
        }
        if j.video == "nvenc" {
            a.extend(
                [
                    "-c:v",
                    "hevc_nvenc",
                    "-preset",
                    "p7",
                    "-tune",
                    "hq",
                    "-rc",
                    "vbr",
                    "-b:v",
                    "0",
                    "-cq",
                    &j.quality.to_string(),
                    "-profile:v",
                    "main10",
                    "-forced-idr",
                    "1",
                ]
                .iter()
                .map(|s| s.to_string()),
            );
        } else {
            let q = 100 - (j.quality as u32 * 2);
            a.extend(
                [
                    "-c:v",
                    "hevc_videotoolbox",
                    "-allow_sw",
                    "0",
                    "-q:v",
                    &q.to_string(),
                    "-profile:v",
                    "main10",
                ]
                .iter()
                .map(|s| s.to_string()),
            );
        }
        for (key, flag) in [
            ("color_primaries", "-color_primaries"),
            ("color_transfer", "-color_trc"),
            ("color_space", "-colorspace"),
            ("color_range", "-color_range"),
        ] {
            if let Some(value) = v[key].as_str() {
                if value != "unknown" {
                    a.extend([flag.into(), value.into()]);
                }
            }
        }
    }
    if j.split_encode {
        a.extend(["-split_encode_mode".into(), "2".into()]);
    }
    Ok(a)
}
#[tauri::command]
async fn inspect(path: String, tool: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || probe(&path, &tool))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn environment() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let ffmpeg=executable("ffmpeg"); let ffprobe=executable("ffprobe");
        let out=command(&ffmpeg).args(["-hide_banner","-encoders"]).output().map_err(|e|format!("Install FFmpeg and ffprobe, or select their executable paths in Settings. {e}"))?;
        let enc=String::from_utf8_lossy(&out.stdout);
        let gpu=command("nvidia-smi").args(["--query-gpu=name","--format=csv,noheader"]).output().ok().filter(|o|o.status.success()).map(|o|String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or("").trim().to_owned()).unwrap_or_default();
        // Conservative desktop allowlist; unknown models retain single-job defaults.
        let dual=enc.contains("hevc_nvenc") && ["NVIDIA GeForce RTX 5070 Ti","NVIDIA GeForce RTX 5080","NVIDIA GeForce RTX 5090","NVIDIA GeForce RTX 4090"].contains(&gpu.as_str());
        let split=dual && command(&ffmpeg).args(["-hide_banner","-h","encoder=hevc_nvenc"]).output().ok().map(|o|String::from_utf8_lossy(&o.stdout).contains("split_encode_mode")).unwrap_or(false);
        Ok(json!({"ffmpeg":ffmpeg,"ffprobe":ffprobe,"nvenc":enc.contains("hevc_nvenc"),"apple":cfg!(target_os="macos")&&enc.contains("hevc_videotoolbox"),"platform":std::env::consts::OS,"dualNvenc":dual,"splitSupported":split,"gpu":gpu}))
    }).await.map_err(|e|e.to_string())?
}
#[tauri::command]
fn reveal_output(path: String) -> Result<(), String> {
    if !Path::new(&path).is_file() {
        return Err("Converted file no longer exists at this location".into());
    }
    tauri_plugin_opener::reveal_item_in_dir(path).map_err(|e| e.to_string())
}
#[tauri::command]
fn status(state: tauri::State<Arc<Queue>>) -> Value {
    json!({"running":state.active.load(Ordering::SeqCst),"jobs":state.entries.lock().unwrap().iter().map(|e|(e.id.clone(),e.state.progress.lock().unwrap().clone())).collect::<std::collections::HashMap<_,_>>()})
}
#[tauri::command]
fn cancel(state: tauri::State<Arc<Queue>>) {
    stop_queue(&state);
}
fn stop_queue(q: &Queue) {
    q.halted.store(true, Ordering::SeqCst);
    for e in q.entries.lock().unwrap().iter() {
        e.state.cancel.store(true, Ordering::SeqCst);
    }
}
fn update(s: &Shared, phase: &str) {
    s.progress.lock().unwrap().phase = phase.into();
}
// Both native calls fail if the destination exists, including on supported SMB shares.
fn rename_exclusive(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: both strings are NUL-terminated and remain alive throughout the call.
        if unsafe { windows_sys::Win32::Storage::FileSystem::MoveFileW(from.as_ptr(), to.as_ptr()) }
            == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStrExt;
        let from = std::ffi::CString::new(from.as_os_str().as_bytes())?;
        let to = std::ffi::CString::new(to.as_os_str().as_bytes())?;
        // SAFETY: both pointers reference valid NUL-terminated paths for this call.
        if unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        fs::hard_link(from, to)?;
        fs::remove_file(from)
    }
}
fn run(j: Job, s: Arc<Shared>) -> Result<String, String> {
    let source_before = fs::metadata(&j.source).map_err(|e| e.to_string())?;
    let p = probe(&j.source, &j.ffprobe)?;
    let mut args = plan(&j, &p)?;
    let source = Path::new(&j.source);
    let folder = if j.replace || j.folder.is_empty() {
        source.parent().ok_or("Source has no folder")?.to_path_buf()
    } else {
        PathBuf::from(&j.folder)
    };
    if !folder.is_dir() {
        return Err("Output folder does not exist".into());
    }
    let stem = source
        .file_stem()
        .ok_or("Invalid filename")?
        .to_string_lossy();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let final_path = if j.replace {
        source.with_extension("mkv")
    } else {
        folder.join(format!(
            "{stem} - {}.mkv",
            if j.video == "copy" { "Remux" } else { "HEVC" }
        ))
    };
    if final_path.exists() && !(j.replace && final_path == source) {
        return Err(format!("Output already exists: {}", final_path.display()));
    }
    let partial = folder.join(format!(".{stem}.{stamp}.part.mkv"));
    let log_path = folder.join(format!("{stem}.{stamp}.conversion.log"));
    let log = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&log_path)
        .map_err(|e| e.to_string())?;
    args.extend([
        "-progress".into(),
        "pipe:1".into(),
        "-nostats".into(),
        partial.to_string_lossy().into_owned(),
    ]);
    update(&s, "Encoding");
    let mut child = command(&j.ffmpeg)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::from(log))
        .spawn()
        .map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().unwrap();
    let reader_state = s.clone();
    let reader = thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some((k, v)) = line.split_once('=') {
                let mut x = reader_state.progress.lock().unwrap();
                match k {
                    "out_time_us" => x.seconds = v.parse::<f64>().unwrap_or(0.0) / 1_000_000.0,
                    "speed" => x.speed = v.into(),
                    _ => {}
                }
            }
        }
    });
    let result = loop {
        if s.cancel.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err(format!(
                "Cancelled. Partial output retained at {}",
                partial.display()
            ));
        }
        if let Some(exit) = child.try_wait().map_err(|e| e.to_string())? {
            break exit;
        }
        thread::sleep(Duration::from_millis(200));
    };
    let _ = reader.join();
    if !result.success() {
        let log = fs::read_to_string(&log_path).unwrap_or_default();
        return Err(format!(
            "Encoding failed. {}\nLog: {}",
            log.chars()
                .rev()
                .take(1400)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>(),
            log_path.display()
        ));
    }
    update(&s, "Checking output");
    let check = probe(&partial.to_string_lossy(), &j.ffprobe)?;
    let a = video(&p)?;
    let b = video(&check)?;
    if j.video != "copy" && j.deinterlace && interlaced(a) && b["field_order"] != "progressive" {
        return Err("Deinterlacing validation failed; original retained".into());
    }
    if (duration(&p) - duration(&check)).abs() > 1.0 {
        return Err("Duration validation failed; partial output retained".into());
    }
    let expected = dimensions(a, j.downscale);
    if (
        b["width"].as_u64().unwrap_or(0),
        b["height"].as_u64().unwrap_or(0),
    ) != expected
    {
        return Err("Raster validation failed".into());
    }
    if a["r_frame_rate"] != b["r_frame_rate"] || a["avg_frame_rate"] != b["avg_frame_rate"] {
        return Err(
            "Frame-rate metadata differs. Output retained for review; original untouched.".into(),
        );
    }
    let output_audio: Vec<_> = streams(&check)?
        .iter()
        .filter(|s| s["codec_type"] == "audio")
        .collect();
    if output_audio.len() != j.audio.len()
        || streams(&check)?
            .iter()
            .filter(|s| s["codec_type"] == "subtitle")
            .count()
            != j.subtitles.len()
        || p["chapters"].as_array().map(Vec::len) != check["chapters"].as_array().map(Vec::len)
    {
        return Err("Track/chapter count validation failed".into());
    }
    for (n, t) in j.audio.iter().enumerate() {
        let original = streams(&p)?.iter().find(|s| s["index"] == t.index).unwrap();
        let got = output_audio[n];
        if original["tags"]["language"] != got["tags"]["language"]
            || original["channels"] != got["channels"]
            || (t.mode == "copy" && original["codec_name"] != got["codec_name"])
            || (t.mode != "copy" && got["codec_name"] != t.mode)
        {
            return Err("Audio mapping/channel-count validation failed".into());
        }
        if let Some(title) = original["tags"]["title"]
            .as_str()
            .or(original["tags"]["name"].as_str())
        {
            if got["tags"]["title"] != title {
                return Err("Track title validation failed".into());
            }
        }
    }
    // Brief decode at start, middle, and end, including every retained audio track.
    for seek in [0.0, duration(&p) * 0.5, (duration(&p) - 15.0).max(0.0)] {
        if s.cancel.load(Ordering::SeqCst) {
            return Err("Cancelled during validation; partial retained".into());
        }
        let stderr = File::options()
            .append(true)
            .open(&log_path)
            .map_err(|e| e.to_string())?;
        let mut child = command(&j.ffmpeg)
            .args([
                "-v",
                "error",
                "-nostdin",
                "-xerror",
                "-ss",
                &seek.to_string(),
                "-protocol_whitelist",
                "file,pipe",
                "-i",
                &partial.to_string_lossy(),
                "-map",
                "0:v:0",
                "-map",
                "0:a?",
                "-t",
                "5",
                "-f",
                "null",
                "-",
            ])
            .stdout(Stdio::null())
            .stderr(stderr)
            .spawn()
            .map_err(|e| e.to_string())?;
        loop {
            if s.cancel.load(Ordering::SeqCst) {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Cancelled; partial retained".into());
            }
            if let Some(exit) = child.try_wait().map_err(|e| e.to_string())? {
                if !exit.success() {
                    return Err(format!("Decode check failed; see {}", log_path.display()));
                }
                break;
            }
            thread::sleep(Duration::from_millis(200));
        }
    }
    if s.cancel.load(Ordering::SeqCst) {
        return Err("Cancelled before publication; partial retained".into());
    }
    let source_after = fs::metadata(source).map_err(|e| e.to_string())?;
    if source_before.len() != source_after.len()
        || source_before.modified().ok() != source_after.modified().ok()
    {
        return Err(
            "Source changed during conversion; output retained and replacement refused".into(),
        );
    }
    // Hard-link publication cannot overwrite a file created by another process.
    let backup = source.with_file_name(format!(
        "{}.backup-{stamp}",
        source.file_name().unwrap().to_string_lossy()
    ));
    if j.replace {
        rename_exclusive(source, &backup).map_err(|e| format!("Cannot back up original: {e}"))?;
    }
    let published = match fs::hard_link(&partial, &final_path) {
        Ok(()) => Ok(()),
        #[cfg(target_os = "macos")]
        Err(e) if e.raw_os_error() == Some(45) => {
            // Never overwrite a destination created by another process during publication.
            rename_exclusive(&partial, &final_path).map_err(|move_error| move_error.to_string())
        }
        Err(e) => Err(e.to_string()),
    };
    if let Err(e) = published {
        if j.replace {
            rename_exclusive(&backup, source).map_err(|restore| {
                format!(
                    "Publish failed: {e}. Restore failed: {restore}. Original is at {}",
                    backup.display()
                )
            })?;
        }
        return Err(format!(
            "Verified output retained at {}. Could not publish: {e}",
            partial.display()
        ));
    }
    // Only this successful job's exact temporary paths; never sweep the movie folder.
    // No success report is created beside the movie. Failures return before cleanup.
    let mut leftovers = Vec::new();
    let mut cleanup_paths = vec![&partial, &log_path];
    if j.replace && !j.keep_backup {
        cleanup_paths.push(&backup);
    }
    for path in cleanup_paths {
        let mut last_error = None;
        for _ in 0..3 {
            match fs::remove_file(path) {
                Ok(()) => {
                    last_error = None;
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    last_error = None;
                    break;
                }
                Err(e) => {
                    last_error = Some(e);
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
        if let Some(e) = last_error {
            leftovers.push(format!("{} ({e})", path.display()));
        }
    }
    if !leftovers.is_empty() {
        s.progress.lock().unwrap().message = format!(
            "Saved successfully, but could not remove cleanup files: {}",
            leftovers.join(", ")
        );
    }
    Ok(final_path.to_string_lossy().into_owned())
}
#[tauri::command]
fn start(
    jobs: Vec<Submission>,
    parallel: usize,
    state: tauri::State<Arc<Queue>>,
) -> Result<(), String> {
    launch_queue(jobs, parallel, state.inner().clone())
}
fn path_key(path: &Path) -> Result<String, String> {
    let resolved = if path.exists() {
        fs::canonicalize(path)
    } else {
        fs::canonicalize(path.parent().ok_or("Missing parent")?)
            .map(|p| p.join(path.file_name().unwrap()))
    }
    .map_err(|e| e.to_string())?;
    let key = resolved.to_string_lossy().into_owned();
    Ok(if cfg!(windows) {
        key.to_lowercase()
    } else {
        key
    })
}
fn validate_batch(jobs: &[Submission], parallel: usize) -> Result<(), String> {
    if !(1..=2).contains(&parallel) || jobs.is_empty() {
        return Err("Choose one or two simultaneous conversions and at least one movie".into());
    }
    let mut ids = std::collections::HashSet::new();
    let mut sources = std::collections::HashSet::new();
    let mut outputs = std::collections::HashSet::new();
    for e in jobs {
        if !ids.insert(&e.id) || !sources.insert(path_key(Path::new(&e.job.source))?) {
            return Err("Duplicate job or source in queue".into());
        }
    }
    for e in jobs {
        let j = &e.job;
        let source = Path::new(&j.source);
        let folder = if j.replace || j.folder.is_empty() {
            source.parent().unwrap().to_path_buf()
        } else {
            PathBuf::from(&j.folder)
        };
        let out = if j.replace {
            source.with_extension("mkv")
        } else {
            folder.join(format!(
                "{} - {}.mkv",
                source.file_stem().unwrap().to_string_lossy(),
                if j.video == "copy" { "Remux" } else { "HEVC" }
            ))
        };
        let key = path_key(&out)?;
        if !outputs.insert(key.clone())
            || (sources.contains(&key) && !(j.replace && key == path_key(source)?))
        {
            return Err(
                "Queue outputs overlap another movie or each other. Choose separate destinations."
                    .into(),
            );
        }
    }
    Ok(())
}
fn launch_queue(jobs: Vec<Submission>, parallel: usize, q: Arc<Queue>) -> Result<(), String> {
    validate_batch(&jobs, parallel)?;
    if q.active
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("A queue is already running".into());
    }
    q.halted.store(false, Ordering::SeqCst);
    *q.entries.lock().unwrap() = jobs
        .into_iter()
        .map(|e| Entry {
            id: e.id,
            job: e.job,
            state: Arc::new(Shared {
                progress: Mutex::new(Progress {
                    phase: "Queued".into(),
                    ..Default::default()
                }),
                cancel: AtomicBool::new(false),
            }),
        })
        .collect();
    thread::spawn(move || {
        let workers: Vec<_> = (0..parallel)
            .map(|_| {
                let q = q.clone();
                thread::spawn(move || loop {
                    let next = {
                        let entries = q.entries.lock().unwrap();
                        if q.halted.load(Ordering::SeqCst) {
                            None
                        } else {
                            entries.iter().find_map(|e| {
                                let mut p = e.state.progress.lock().unwrap();
                                if p.phase != "Queued" {
                                    return None;
                                }
                                p.running = true;
                                p.phase = "Inspecting".into();
                                Some((e.job.clone(), e.state.clone()))
                            })
                        }
                    };
                    let Some((job, s)) = next else { break };
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        run(job, s.clone())
                    }))
                    .unwrap_or_else(|_| {
                        Err("Conversion worker failed unexpectedly; recovery files retained".into())
                    });
                    let mut p = s.progress.lock().unwrap();
                    p.running = false;
                    match result {
                        Ok(path) => {
                            p.phase = "Complete".into();
                            p.output = path;
                            if p.message.is_empty() {
                                p.message = "Verified and saved.".into()
                            }
                        }
                        Err(e) => {
                            p.phase = if s.cancel.load(Ordering::SeqCst) {
                                "Cancelled"
                            } else {
                                "Failed"
                            }
                            .into();
                            p.message = e
                        }
                    }
                })
            })
            .collect();
        for w in workers {
            let _ = w.join();
        }
        for e in q.entries.lock().unwrap().iter() {
            let mut p = e.state.progress.lock().unwrap();
            if p.phase == "Queued" {
                p.phase = "Ready".into();
                p.message = "Not started; queue stopped.".into();
            }
        }
        q.active.store(false, Ordering::SeqCst);
    });
    Ok(())
}
fn main() {
    let queue = Arc::new(Queue::default());
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(queue.clone())
        .manage(remote::RemoteClientState::default())
        .manage(remote::RemoteServerState::default())
        .invoke_handler(tauri::generate_handler![
            environment,
            inspect,
            start,
            status,
            cancel,
            reveal_output,
            remote::enable_remote,
            remote::disable_remote,
            remote::remote_connect,
            remote::remote_disconnect,
            remote::remote_call
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                use tauri::Manager;
                let state = window.state::<Arc<Queue>>();
                if state.active.load(Ordering::SeqCst) {
                    api.prevent_close();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("Could not start Movie Harbor");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exclusive_rename_never_overwrites() {
        let dir = std::env::temp_dir().join(format!(
            "harbor-exclusive-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let source = dir.join("source");
        let dest = dir.join("destination");
        fs::write(&source, b"original").unwrap();
        fs::write(&dest, b"other").unwrap();
        assert!(rename_exclusive(&source, &dest).is_err());
        assert_eq!(fs::read(&source).unwrap(), b"original");
        assert_eq!(fs::read(&dest).unwrap(), b"other");
        let available = dir.join("available");
        rename_exclusive(&source, &available).unwrap();
        assert!(!source.exists());
        assert_eq!(fs::read(available).unwrap(), b"original");
    }
    fn fixture() -> Value {
        json!({"streams":[{"index":0,"codec_type":"video","width":3840,"height":1608,"r_frame_rate":"24000/1001","avg_frame_rate":"24000/1001"},{"index":2,"codec_type":"audio","channels":6,"tags":{"title":"Original mix","language":"eng"}}]})
    }
    fn job() -> Job {
        Job {
            deinterlace: true,
            keep_backup: true,
            split_encode: false,
            repair_audio_timestamps: false,
            source: "a movie.mkv".into(),
            folder: "".into(),
            replace: false,
            video: "nvenc".into(),
            downscale: true,
            quality: 24,
            audio: vec![Audio {
                index: 2,
                mode: "aac".into(),
            }],
            subtitles: vec![],
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
        }
    }
    #[test]
    fn eac3_channel_guard() {
        let mut j = job();
        j.audio[0].mode = "eac3".into();
        let mut p = fixture();
        p["streams"][1]["channel_layout"] = json!("5.1(side)");
        assert!(plan(&j, &p).unwrap().contains(&"640k".into()));
        p["streams"][1]["channels"] = json!(8);
        p["streams"][1]["channel_layout"] = json!("7.1");
        assert!(plan(&j, &p).is_err());
    }
    #[test]
    fn split_is_explicit_and_nvidia_only() {
        let mut j = job();
        assert!(!plan(&j, &fixture())
            .unwrap()
            .contains(&"-split_encode_mode".into()));
        j.split_encode = true;
        assert!(plan(&j, &fixture())
            .unwrap()
            .contains(&"-split_encode_mode".into()));
        j.video = "apple".into();
        assert!(plan(&j, &fixture()).is_err());
    }
    #[test]
    fn interlace_policy() {
        let mut p = fixture();
        p["streams"][0]["field_order"] = json!("tt");
        let mut j = job();
        assert!(plan(&j, &p)
            .unwrap()
            .iter()
            .any(|x| x.contains("bwdif=mode=send_frame")));
        assert!(plan(&j, &p)
            .unwrap()
            .iter()
            .any(|x| x.contains("setfield=prog")));
        assert!(plan(&j, &p)
            .unwrap()
            .windows(2)
            .any(|x| x == ["-field_order", "progressive"]));
        j.deinterlace = false;
        assert!(plan(&j, &p).is_err());
        p["streams"][0]["field_order"] = json!("unknown");
        assert!(plan(&j, &p).is_ok());
        j.video = "copy".into();
        j.downscale = false;
        p["streams"][0]["field_order"] = json!("tt");
        assert!(!plan(&j, &p).unwrap().iter().any(|x| x.contains("bwdif")));
    }
    #[test]
    #[ignore = "requires local FFmpeg and NVIDIA hardware"]
    fn actual_interlaced_conversion() {
        let dir = std::env::temp_dir().join(format!(
            "movie-harbor-interlaced-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let source = dir.join("interlaced.mkv");
        let ff = executable("ffmpeg");
        let fp = executable("ffprobe");
        let result = command(&ff)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=720x576:rate=50",
                "-t",
                "2",
                "-vf",
                "tinterlace=mode=interleave_top",
                "-c:v",
                "mpeg2video",
                "-flags",
                "+ilme+ildct",
                "-top",
                "1",
                source.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let p = probe(source.to_str().unwrap(), &fp).unwrap();
        assert!(interlaced(video(&p).unwrap()));
        let mut j = job();
        j.source = source.to_string_lossy().into_owned();
        j.ffmpeg = ff;
        j.ffprobe = fp.clone();
        j.audio.clear();
        let output = run(j, Arc::new(Shared::default())).unwrap();
        assert_eq!(
            probe(&output, &fp).unwrap()["streams"][0]["field_order"],
            "progressive"
        );
        let count = |path: &str| {
            let o = command(&fp)
                .args([
                    "-v",
                    "error",
                    "-count_frames",
                    "-select_streams",
                    "v:0",
                    "-show_entries",
                    "stream=nb_read_frames",
                    "-of",
                    "csv=p=0",
                    path,
                ])
                .output()
                .unwrap();
            assert!(o.status.success());
            String::from_utf8(o.stdout)
                .unwrap()
                .trim()
                .trim_end_matches(',')
                .parse::<u64>()
                .unwrap()
        };
        assert_eq!(count(source.to_str().unwrap()), count(&output));
    }
    #[test]
    #[ignore = "requires local FFmpeg and NVIDIA hardware"]
    fn parallel_split_and_cancel() {
        let dir = std::env::temp_dir().join(format!(
            "movie-harbor-parallel-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let source = dir.join("one.mkv");
        let ff = executable("ffmpeg");
        let result = command(&ff)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=1920x1080:rate=24",
                "-t",
                "3",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                source.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(result.status.success());
        let mut first = job();
        first.source = source.to_string_lossy().into_owned();
        first.audio.clear();
        first.ffmpeg = ff;
        first.ffprobe = executable("ffprobe");
        first.split_encode = true;
        let second_path = dir.join("two.mkv");
        fs::copy(&source, &second_path).unwrap();
        let mut second = first.clone();
        second.source = second_path.to_string_lossy().into_owned();
        second.split_encode = false;
        let q = Arc::new(Queue::default());
        let duplicate = vec![
            Submission {
                id: "a".into(),
                job: first.clone(),
            },
            Submission {
                id: "b".into(),
                job: first.clone(),
            },
        ];
        assert!(validate_batch(&duplicate, 2).is_err());
        assert!(validate_batch(&duplicate, 3).is_err());
        launch_queue(
            vec![
                Submission {
                    id: "a".into(),
                    job: first.clone(),
                },
                Submission {
                    id: "b".into(),
                    job: second.clone(),
                },
            ],
            2,
            q.clone(),
        )
        .unwrap();
        let mut saw_two = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(90);
        while q.active.load(Ordering::SeqCst) {
            assert!(std::time::Instant::now() < deadline);
            let count = q
                .entries
                .lock()
                .unwrap()
                .iter()
                .filter(|e| e.state.progress.lock().unwrap().running)
                .count();
            saw_two |= count == 2;
            thread::sleep(Duration::from_millis(10));
        }
        assert!(saw_two);
        for e in q.entries.lock().unwrap().iter() {
            let p = e.state.progress.lock().unwrap();
            assert_eq!(p.phase, "Complete", "{}", p.message);
        }
        for (j, name) in [
            (&mut first, "cancel-one.mkv"),
            (&mut second, "cancel-two.mkv"),
        ] {
            let path = dir.join(name);
            fs::copy(&source, &path).unwrap();
            j.source = path.to_string_lossy().into_owned();
        }
        launch_queue(
            vec![
                Submission {
                    id: "c".into(),
                    job: first,
                },
                Submission {
                    id: "d".into(),
                    job: second,
                },
            ],
            2,
            q.clone(),
        )
        .unwrap();
        while q
            .entries
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.state.progress.lock().unwrap().running)
            .count()
            < 2
        {
            assert!(std::time::Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        stop_queue(&q);
        while q.active.load(Ordering::SeqCst) {
            assert!(std::time::Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        for e in q.entries.lock().unwrap().iter() {
            assert!(!e.state.progress.lock().unwrap().running);
            assert!(Path::new(&e.job.source).exists());
        }
    }
    #[test]
    fn aspect_ratio_and_no_upscale() {
        assert_eq!(dimensions(&fixture()["streams"][0], true), (1920, 804));
        assert_eq!(
            dimensions(&json!({"width":1280,"height":720}), true),
            (1280, 720)
        );
    }
    #[test]
    fn title_and_passthrough() {
        let args = plan(&job(), &fixture()).unwrap();
        assert!(args.contains(&"title=Original mix".into()));
        assert!(args.contains(&"passthrough".into()));
        assert!(!args.contains(&"-r".into()));
        assert!(args.contains(&"512k".into()));
    }
    #[test]
    fn reject_hdr_and_invalid_mapping() {
        let mut p = fixture();
        p["streams"][0]["color_transfer"] = json!("smpte2084");
        assert!(plan(&job(), &p).is_err());
        let mut j = job();
        j.audio[0].index = 999;
        assert!(plan(&j, &fixture()).is_err());
    }
    #[test]
    fn repair_is_opt_in_for_all_audio() {
        let mut j = job();
        j.audio[0].mode = "copy".into();
        assert!(!plan(&j, &fixture()).unwrap().contains(&"-bsf:a:0".into()));
        j.repair_audio_timestamps = true;
        let args = plan(&j, &fixture()).unwrap();
        assert!(args.contains(&"-xerror".into()));
        assert!(args.contains(&"-bsf:a:0".into()));
        assert!(!args.iter().any(|x| x.starts_with("-bsf:v")));
        for mode in ["aac", "eac3"] {
            j.audio[0].mode = mode.into();
            let mut p = fixture();
            p["streams"][1]["channel_layout"] = json!("5.1(side)");
            assert!(plan(&j, &p).unwrap().contains(&"-bsf:a:0".into()));
        }
    }
    #[test]
    #[ignore = "requires local FFmpeg"]
    fn failed_movie_does_not_stop_next_job() {
        let dir = std::env::temp_dir().join(format!(
            "movie-harbor-queue-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let source = dir.join("good.mkv");
        let ff = executable("ffmpeg");
        let fp = executable("ffprobe");
        let created = command(&ff)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x180:rate=24",
                "-t",
                "1",
                "-c:v",
                "libx264",
                source.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(
            created.status.success(),
            "{}",
            String::from_utf8_lossy(&created.stderr)
        );
        let failed_source = dir.join("bad.mkv");
        fs::copy(&source, &failed_source).unwrap();
        let mut good = job();
        good.source = source.to_string_lossy().into_owned();
        good.video = "copy".into();
        good.downscale = false;
        good.audio.clear();
        good.ffmpeg = ff;
        good.ffprobe = fp;
        let mut bad = good.clone();
        bad.source = failed_source.to_string_lossy().into_owned();
        bad.audio.push(Audio {
            index: 999,
            mode: "copy".into(),
        });
        let q = Arc::new(Queue::default());
        launch_queue(
            vec![
                Submission {
                    id: "bad".into(),
                    job: bad,
                },
                Submission {
                    id: "good".into(),
                    job: good,
                },
            ],
            1,
            q.clone(),
        )
        .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while q.active.load(Ordering::SeqCst) {
            assert!(std::time::Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        let entries = q.entries.lock().unwrap();
        assert_eq!(entries[0].state.progress.lock().unwrap().phase, "Failed");
        assert_eq!(entries[1].state.progress.lock().unwrap().phase, "Complete");
        assert!(failed_source.exists());
        assert!(dir.join("good - Remux.mkv").exists());
    }
    #[test]
    #[ignore = "requires local FFmpeg and NVIDIA hardware"]
    fn actual_conversion_and_replacement() {
        let dir = std::env::temp_dir().join(format!(
            "movie-harbor-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        fs::create_dir(&dir).unwrap();
        let source = dir.join("sample.mkv");
        let ff = executable("ffmpeg");
        let fp = executable("ffprobe");
        let result = command(&ff)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x180:rate=24000/1001",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000",
                "-t",
                "3",
                "-c:v",
                "libx264",
                "-c:a",
                "flac",
                "-metadata:s:a:0",
                "title=Original mix",
                "-metadata:s:a:0",
                "language=eng",
                source.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let original = fs::read(&source).unwrap();
        let mut j = job();
        j.source = source.to_string_lossy().into_owned();
        j.audio[0].index = 1;
        j.ffmpeg = ff;
        j.ffprobe = fp;
        j.replace = true;
        let output = run(j, Arc::new(Shared::default())).unwrap();
        assert_eq!(Path::new(&output), source);
        let backup = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.to_string_lossy().contains(".backup-"))
            .unwrap();
        assert_eq!(fs::read(backup).unwrap(), original);
        // A second replacement opts out of retention; only its own rollback file is removed.
        let mut next = job();
        next.source = source.to_string_lossy().into_owned();
        next.video = "copy".into();
        next.downscale = false;
        next.replace = true;
        next.keep_backup = false;
        next.audio = vec![Audio {
            index: 1,
            mode: "copy".into(),
        }];
        next.ffmpeg = executable("ffmpeg");
        next.ffprobe = executable("ffprobe");
        run(next, Arc::new(Shared::default())).unwrap();
        assert_eq!(
            fs::read_dir(&dir)
                .unwrap()
                .filter(|e| e
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains(".backup-"))
                .count(),
            1,
            "Existing backup kept; new backup removed"
        );
        println!("Integration artifacts: {}", dir.display());
    }
    #[test]
    #[ignore = "requires local FFmpeg"]
    fn actual_eac3_conversion() {
        let dir = std::env::temp_dir().join(format!(
            "movie-harbor-eac3-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        fs::create_dir(&dir).unwrap();
        let source = dir.join("sample.mkv");
        let ff = executable("ffmpeg");
        let result = command(&ff)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x180:rate=24",
                "-f",
                "lavfi",
                "-i",
                "anullsrc=r=48000:cl=5.1(side)",
                "-t",
                "3",
                "-c:v",
                "libx264",
                "-c:a",
                "flac",
                "-metadata:s:a:0",
                "title=Original surround",
                "-metadata:s:a:0",
                "language=eng",
                source.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(result.status.success());
        let mut j = job();
        j.source = source.to_string_lossy().into_owned();
        j.ffmpeg = ff;
        j.ffprobe = executable("ffprobe");
        j.video = "copy".into();
        j.downscale = false;
        j.audio = vec![Audio {
            index: 1,
            mode: "eac3".into(),
        }];
        let output = run(j, Arc::new(Shared::default())).unwrap();
        let p = probe(&output, &executable("ffprobe")).unwrap();
        assert_eq!(p["streams"][1]["codec_name"], "eac3");
        assert_eq!(p["streams"][1]["channels"], 6);
        assert_eq!(p["streams"][1]["tags"]["title"], "Original surround");
        assert!(source.exists());
        assert_eq!(
            fs::read_dir(&dir).unwrap().count(),
            2,
            "Only original and completed output should remain"
        );
    }
}
