import React, { useEffect, useRef, useState } from "react";

import { createRoot } from "react-dom/client";
import appLicense from "../LICENSE?raw";

import { invoke } from "@tauri-apps/api/core";

import { getCurrentWebview } from "@tauri-apps/api/webview";

import { open } from "@tauri-apps/plugin-dialog";

import {
  Film,
  Plus,
  Play,
  Square,
  FolderOpen,
  Check,
  SlidersHorizontal,
  X,
  AudioLines,
  Captions,
  ArrowDownToLine,
} from "lucide-react";

import "./style.css";

import { recommendedAudio, canEac3 } from "./audioPolicy";

type Stream = {
  index: number;
  codec_type: string;
  codec_name: string;
  profile?: string;
  field_order?: string;
  width?: number;
  height?: number;
  channels?: number;
  channel_layout?: string;
  r_frame_rate?: string;
  avg_frame_rate?: string;
  color_transfer?: string;
  side_data_list?: unknown[];
  tags?: { language?: string; title?: string; name?: string };
  disposition?: { attached_pic?: number; timed_thumbnails?: number };
};

type Probe = {
  streams: Stream[];
  format: { duration: string; size: string };
  chapters: unknown[];
};

type Movie = {
  id: string;
  path: string;
  probe: Probe;
  audio: Record<number, string>;
  subtitles: number[];
  video: string;
  downscale: boolean;
  quality: number;
  deinterlace?: boolean;
  splitEncode?: boolean;
  repairAudioTimestamps: boolean;
  recommended?: boolean;
  manualAudio?: Record<number, string>;
  state: string;
  message?: string;
  output?: string;
};

type Progress = {
  running: boolean;
  phase: string;
  seconds: number;
  speed: string;
  message: string;
  output: string;
};

type Env = {
  ffmpeg: string;
  ffprobe: string;
  nvenc: boolean;
  apple: boolean;
  platform: string;
  dualNvenc?: boolean;
  splitSupported?: boolean;
  gpu?: string;
};

const mainVideo = (m: Movie) =>
  m.probe.streams.find(
    (s) =>
      s.codec_type === "video" &&
      !s.disposition?.attached_pic &&
      !s.disposition?.timed_thumbnails,
  )!;

const filename = (path: string) => path.split(/[\\/]/).pop() || path;

const size = (n: number) => (n / 1024 ** 3).toFixed(2) + " GB";

function App() {
  const [licensesOpen, setLicensesOpen] = useState(false);
  const [licenseText, setLicenseText] = useState("");
  const [movies, setMovies] = useState<Movie[]>([]),
    [selected, setSelected] = useState(""),
    [env, setEnv] = useState<Env>({
      ffmpeg: "ffmpeg",
      ffprobe: "ffprobe",
      nvenc: false,
      apple: false,
      platform: "",
    });

  const [folder, setFolder] = useState(""),
    [replace, setReplace] = useState(true),
    [settings, setSettings] = useState(false),
    [error, setError] = useState(""),
    [loading, setLoading] = useState(false),
    [active, setActive] = useState(false),
    [jobProgress, setJobProgress] = useState<Record<string, Progress>>({}),
    [parallel, setParallel] = useState(1),
    [keepBackup, setKeepBackup] = useState(false);

  const [remote, setRemote] = useState(false),
    [localPlatform, setLocalPlatform] = useState("");
  const [connection, setConnection] = useState({
    host: "",
    user: "",
    key_path: "",
    local_root: "",
    token: "",
  });
  const [serverInfo, setServerInfo] = useState<{
      token: string;
      root: string;
    } | null>(null),
    [showToken, setShowToken] = useState(false);
  const localDraft = useRef<{
    movies: Movie[];
    selected: string;
    folder: string;
    replace: boolean;
    keepBackup: boolean;
    parallel: number;
  } | null>(null);
  async function chooseConnectionPath(field: "key_path" | "local_root") {
    const path = await open({
      directory: field === "local_root",
      multiple: false,
    });
    if (typeof path === "string")
      setConnection((c) => ({ ...c, [field]: path }));
  }
  async function toggleServer() {
    try {
      if (serverInfo) {
        await invoke("disable_remote");
        setServerInfo(null);
        setShowToken(false);
      } else {
        const root = await open({ directory: true, multiple: false });
        if (typeof root === "string")
          setServerInfo(await invoke("enable_remote", { root }));
      }
    } catch (e) {
      setError(String(e));
    }
  }
  async function connectMac() {
    try {
      setLoading(true);
      setError("");
      const next = await invoke<Env>("remote_connect", { config: connection });
      const saved = await invoke<
        Array<{
          id: string;
          job: {
            source: string;
            audio: { index: number; mode: string }[];
            subtitles: number[];
            video: string;
            downscale: boolean;
            quality: number;
            deinterlace: boolean;
            split_encode: boolean;
            repair_audio_timestamps: boolean;
            replace: boolean;
            keep_backup: boolean;
          };
          progress: Progress;
        }>
      >("remote_call", { action: "queue", payload: {} });
      const restored: Movie[] = [];
      for (const entry of saved) {
        const path = entry.progress.phase === "Complete" && entry.progress.output
          ? entry.progress.output : entry.job.source;
        const probe = await invoke<Probe>("remote_call", {
          action: "inspect",
          payload: { path },
        });
        restored.push({
          id: entry.id,
          path,
          probe,
          audio: Object.fromEntries(
            entry.job.audio.map((a) => [a.index, a.mode]),
          ),
          subtitles: entry.job.subtitles,
          video: entry.job.video,
          downscale: entry.job.downscale,
          quality: entry.job.quality,
          deinterlace: entry.job.deinterlace,
          splitEncode: entry.job.split_encode,
          repairAudioTimestamps: entry.job.repair_audio_timestamps,
          recommended: false,
          state: entry.progress.phase,
          message: entry.progress.message,
          output: entry.progress.output,
        });
      }
      localDraft.current = {
        movies,
        selected,
        folder,
        replace,
        keepBackup,
        parallel,
      };
      setEnv(next);
      setParallel(1);
      setMovies(restored);
      setSelected(restored[0]?.id || "");
      setRemote(true);
      setFolder("");
      if (saved.length) {
        setReplace(saved[0].job.replace);
        setKeepBackup(saved[0].job.keep_backup);
        setJobProgress(
          Object.fromEntries(saved.map((e) => [e.id, e.progress])),
        );
      }
      if (
        saved.some((e) => e.progress.running || e.progress.phase === "Queued")
      ) {
        setActive(true);
        void watchRemoteQueue();
      }
    } catch (e) {
      await invoke("remote_disconnect").catch(() => {});
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }
  async function watchRemoteQueue() {
    let snapshot: { running: boolean; jobs: Record<string, Progress> };
    do {
      await new Promise((r) => setTimeout(r, 600));
      try {
        snapshot = await invoke("remote_call", {
          action: "status",
          payload: {},
        });
      } catch (e) {
        setError(String(e));
        setActive(false);
        return;
      }
      setJobProgress(snapshot.jobs);
      setMovies((ms) =>
        ms.map((m) => {
          const p = snapshot.jobs[m.id];
          return p
            ? {
                ...m,
                state: p.phase,
                output: p.output || m.output,
                message: p.phase === "Complete" ? p.output : p.message,
              }
            : m;
        }),
      );
    } while (snapshot.running);
    setActive(false);
  }
  async function disconnectMac() {
    try {
      await invoke("remote_disconnect");
      const next = await invoke<Env>("environment");
      setEnv(next);
      setParallel(localDraft.current?.parallel ?? (next.dualNvenc ? 2 : 1));
      setMovies(localDraft.current?.movies ?? []);
      setSelected(localDraft.current?.selected ?? "");
      setReplace(localDraft.current?.replace ?? true);
      setKeepBackup(localDraft.current?.keepBackup ?? false);
      setRemote(false);
      setFolder(localDraft.current?.folder ?? "");
      setJobProgress({});
      localDraft.current = null;
    } catch (e) {
      setError(String(e));
    }
  }

  const [context, setContext] = useState<{
    id: string;
    x: number;
    y: number;
  } | null>(null);

  useEffect(() => {
    if (!context) return;
    const close = () => setContext(null);
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
    };
    window.addEventListener("click", close);
    window.addEventListener("keydown", key);
    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("keydown", key);
    };
  }, [context]);

  async function reveal(id: string) {
    const path = movies.find((m) => m.id === id)?.output;
    if (path)
      try {
        await invoke("reveal_output", { path });
      } catch (e) {
        setError(String(e));
      }
    setContext(null);
  }

  const call = <T,>(action: string, payload: Record<string, unknown> = {}) =>
    remote
      ? invoke<T>("remote_call", { action, payload })
      : invoke<T>(action, payload);

  const current = movies.find((m) => m.id === selected);
  const locked = active || loading;

  useEffect(() => {
    invoke<Env>("environment")
      .then((e) => {
        setEnv(e);
        setLocalPlatform(e.platform);
        setParallel(e.dualNvenc ? 2 : 1);
      })
      .catch((e) => {
        setError(String(e));
        setSettings(true);
      });
  }, []);

  const edit = (patch: Partial<Movie>) =>
    setMovies((ms) =>
      ms.map((m) =>
        m.id === selected
          ? { ...m, ...patch, state: "Ready", message: undefined }
          : m,
      ),
    );

  const [dragging, setDragging] = useState(false);
  const importBusy = useRef(false);

  async function addPaths(paths: string[]) {
    if (active || importBusy.current) return;

    importBusy.current = true;
    setLoading(true);
    setError("");

    const additions: Movie[] = [];
    const errors: string[] = [];

    const key = (p: string) =>
      env.platform === "windows" ? p.replaceAll("/", "\\").toLowerCase() : p;

    const seen = new Set(movies.map((m) => key(m.path)));

    try {
      for (const path of paths) {
        if (seen.has(key(path))) continue;
        seen.add(key(path));

        if (!/\.(mkv|mp4|m4v|mov|avi|ts|m2ts|webm)$/i.test(path)) {
          errors.push(filename(path) + ": unsupported file type");
          continue;
        }

        try {
          const probe = await call<Probe>("inspect", {
            path,
            tool: env.ffprobe,
          });

          if (
            !probe.streams.some(
              (s) => s.codec_type === "video" && !s.disposition?.attached_pic,
            )
          )
            throw new Error("No main video");

          additions.push({
            id: crypto.randomUUID(),
            path,
            probe,
            audio: Object.fromEntries(
              probe.streams
                .filter((s) => s.codec_type === "audio")
                .map((s) => [s.index, recommendedAudio(s)]),
            ),
            recommended: true,
            manualAudio: Object.fromEntries(
              probe.streams
                .filter((s) => s.codec_type === "audio")
                .map((s) => [s.index, "copy"]),
            ),
            splitEncode: !!env.splitSupported,
            subtitles: probe.streams
              .filter((s) => s.codec_type === "subtitle")
              .map((s) => s.index),
            video: env.apple ? "apple" : env.nvenc ? "nvenc" : "copy",
            downscale: false,
            quality: 24,
            repairAudioTimestamps: true,
            state: "Ready",
          });
        } catch (e) {
          errors.push(filename(path) + ": " + String(e));
        }
      }

      setMovies((ms) => [...ms, ...additions]);
      if (additions.length) setSelected(additions[0].id);
      if (errors.length) setError(errors.join("\n"));
    } finally {
      importBusy.current = false;
      setLoading(false);
    }
  }

  async function add() {
    try {
      const paths = await open({
        multiple: true,
        filters: [
          {
            name: "Movies",
            extensions: [
              "mkv",
              "mp4",
              "m4v",
              "mov",
              "avi",
              "ts",
              "m2ts",
              "webm",
            ],
          },
        ],
      });
      if (paths) await addPaths(Array.isArray(paths) ? paths : [paths]);
    } catch (e) {
      setError(String(e));
    }
  }

  const dropHandler = useRef(addPaths);
  dropHandler.current = addPaths;

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;

    getCurrentWebview()
      .onDragDropEvent(({ payload }) => {
        if (disposed) return;

        if (payload.type === "enter" || payload.type === "over")
          setDragging(true);
        else {
          setDragging(false);
          if (payload.type === "drop") void dropHandler.current(payload.paths);
        }
      })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      })
      .catch((e) => {
        if (!disposed) setError("Drag-and-drop unavailable: " + String(e));
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  async function chooseFolder() {
    const path = await open({ directory: true });
    if (typeof path === "string") setFolder(path);
  }

  async function chooseTool(key: "ffmpeg" | "ffprobe") {
    const path = await open({ multiple: false });
    if (typeof path === "string") setEnv((e) => ({ ...e, [key]: path }));
  }

  async function run() {
    setError("");
    setActive(true);
    setJobProgress({});

    try {
      const ready = movies.filter((m) => m.state === "Ready");

      await call("start", {
        parallel,
        jobs: ready.map((movie) => ({
          id: movie.id,
          job: {
            source: movie.path,
            folder,
            replace,
            keep_backup: keepBackup,
            deinterlace: movie.deinterlace !== false,
            video: movie.video,
            downscale: movie.video !== "copy" && movie.downscale,
            quality: movie.quality,
            split_encode: movie.video === "nvenc" && !!movie.splitEncode,
            repair_audio_timestamps: movie.repairAudioTimestamps,
            audio: Object.entries(movie.audio)
              .filter(([, mode]) => mode !== "off")
              .map(([index, mode]) => ({ index: Number(index), mode })),
            subtitles: movie.subtitles,
            ffmpeg: env.ffmpeg,
            ffprobe: env.ffprobe,
          },
        })),
      });

      let snapshot: { running: boolean; jobs: Record<string, Progress> };

      do {
        await new Promise((r) => setTimeout(r, 600));
        snapshot = await call("status");
        setJobProgress(snapshot.jobs);

        setMovies((ms) =>
          ms.map((m) => {
            const p = snapshot.jobs[m.id];
            return p
              ? {
                  ...m,
                  state: p.phase,
                  output: p.output || m.output,
                  message:
                    p.phase === "Complete"
                      ? p.output +
                        (p.message.startsWith("Saved successfully, but")
                          ? " — " + p.message
                          : "")
                      : p.message,
                }
              : m;
          }),
        );
      } while (snapshot.running);

      const failures = Object.values(snapshot.jobs).filter(
        (p) => p.phase === "Failed" || p.phase === "Cancelled",
      );
      if (failures.length) setError(failures.map((p) => p.message).join("\n"));
    } catch (e) {
      setError(String(e));
    } finally {
      setActive(false);
    }
  }

  async function cancel() {
    await call("cancel");
  }

  function toggleRecommended(enabled: boolean) {
    if (!current) return;
    if (enabled) {
      edit({
        recommended: true,
        manualAudio: { ...current.audio },
        audio: Object.fromEntries(
          current.probe.streams
            .filter((s) => s.codec_type === "audio")
            .map((s) => [
              s.index,
              current.audio[s.index] === "off" ? "off" : recommendedAudio(s),
            ]),
        ),
      });
    } else {
      edit({
        recommended: false,
        audio: current.manualAudio || current.audio,
        manualAudio: undefined,
      });
    }
  }

  const v = current ? mainVideo(current) : undefined;
  const isHdr =
    v &&
    (["smpte2084", "arib-std-b67"].includes(v.color_transfer || "") ||
      JSON.stringify(v.side_data_list ?? []).includes("DOVI"));

  const progress = current ? jobProgress[current.id] : undefined;

  const pct =
    current && progress
      ? Math.min(
          100,
          (progress.seconds / Number(current.probe.format.duration)) * 100,
        )
      : 0;

  return (
    <div
      className="shell"
      onDragOver={(e) => e.preventDefault()}
      onDrop={(e) => e.preventDefault()}
    >
      {dragging && (
        <div className="drop-overlay" role="status">
          <div>
            <Plus size={36} />
            <strong>
              {locked
                ? "Please wait for the current work to finish"
                : "Drop movies to add to the queue"}
            </strong>
            <span>Main window or sidebar · multiple files supported</span>
          </div>
        </div>
      )}
      <header>
        <div className="brand">
          <img className="app-icon" src="/movie-harbor.png" alt="" />
          <div>
            Movie Harbor<small>YOUR LIBRARY, LIGHTER.</small>
          </div>
        </div>
        <button className="quiet" onClick={() => setSettings(!settings)}>
          <SlidersHorizontal size={16} /> Settings
        </button>
      </header>

      <div className="workspace">
        <aside>
          <button className="add" disabled={locked} onClick={add}>
            <Plus size={17} />
            {loading ? "Reading movies…" : "Add movies"}
          </button>
          <div className="queue">
            <table className="movie-table">
              <thead>
                <tr>
                  <th>Movie</th>
                  <th>Status</th>
                </tr>
              </thead>
              <tbody>
                {movies.map((m) => (
                  <tr
                    key={m.id}
                    className={"movie " + (selected === m.id ? "selected" : "")}
                    style={
                      {
                        "--movie-progress": `${m.state === "Complete" ? 100 : Math.max(0, Math.min(100, ((jobProgress[m.id]?.seconds || 0) / Number(m.probe.format.duration)) * 100))}%`,
                      } as React.CSSProperties
                    }
                    tabIndex={0}
                    aria-selected={selected === m.id}
                    onClick={() => setSelected(m.id)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") setSelected(m.id);
                      if (e.key === "F10" && e.shiftKey) {
                        e.preventDefault();
                        const r = e.currentTarget.getBoundingClientRect();
                        setContext({ id: m.id, x: r.left + 20, y: r.bottom });
                      }
                    }}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      setSelected(m.id);
                      setContext({
                        id: m.id,
                        x: Math.min(e.clientX, window.innerWidth - 270),
                        y: Math.min(e.clientY, window.innerHeight - 50),
                      });
                    }}
                  >
                    <td title={filename(m.path)}>
                      {filename(m.path).replace(/\.[^.]+$/, "")}
                    </td>
                    <td>
                      {jobProgress[m.id]?.running
                        ? Math.min(
                            100,
                            (jobProgress[m.id].seconds /
                              Number(m.probe.format.duration)) *
                              100,
                          ).toFixed(0) + "%"
                        : m.state}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
            {!movies.length && (
              <p className="empty-queue">
                Drag movies here
                <br />
                or use Add movies.
              </p>
            )}
          </div>
          <div className="aside-foot">
            <span className="dot" /> {remote ? "Remote Mac · " : ""}
            {parallel === 1
              ? "One movie at a time"
              : "Up to two movies at once"}
            <small>Each source gets its own track map.</small>
          </div>
        </aside>

        <main>
          {settings && (
            <section className="settings">
              <h3>Settings</h3>
              <details className="remote-settings">
                <summary>Remote conversion</summary>
                {localPlatform === "macos" ? (
                  <>
                    <p>
                      Share one folder with an authenticated SSH client. Remote
                      control is off until enabled and stops when this app
                      closes.
                    </p>
                    <button disabled={locked} onClick={toggleServer}>
                      {serverInfo
                        ? "Disable remote control"
                        : "Enable remote control…"}
                    </button>
                    {serverInfo && (
                      <>
                        <p>Shared folder: {serverInfo.root}</p>
                        <label>
                          Access token
                          <input
                            readOnly
                            type={showToken ? "text" : "password"}
                            value={serverInfo.token}
                            onFocus={(e) => e.currentTarget.select()}
                          />
                        </label>
                        <button
                          className="quiet"
                          onClick={() => setShowToken(!showToken)}
                        >
                          {showToken ? "Hide token" : "Show token"}
                        </button>
                        <small>
                          Share this token only with your own SSH client. A new
                          token is generated each time you enable remote
                          control.
                        </small>
                      </>
                    )}
                  </>
                ) : (
                  <>
                    <p>
                      Enable remote control in the Mac app first. Both computers
                      must have the same media folder mounted. Connection
                      details are kept only for this session.
                    </p>
                    {remote ? (
                      <button disabled={locked} onClick={disconnectMac}>
                        Disconnect Mac
                      </button>
                    ) : (
                      <>
                        <label>
                          Mac hostname
                          <input
                            disabled={locked}
                            value={connection.host}
                            placeholder="mac.local"
                            onChange={(e) =>
                              setConnection({
                                ...connection,
                                host: e.target.value,
                              })
                            }
                          />
                        </label>
                        <label>
                          SSH username
                          <input
                            disabled={locked}
                            value={connection.user}
                            onChange={(e) =>
                              setConnection({
                                ...connection,
                                user: e.target.value,
                              })
                            }
                          />
                        </label>
                        <label>
                          SSH private key
                          <div className="flex gap-2">
                            <input
                              disabled={locked}
                              readOnly
                              value={connection.key_path}
                            />
                            <button
                              disabled={locked}
                              onClick={() => chooseConnectionPath("key_path")}
                            >
                              Browse
                            </button>
                          </div>
                        </label>
                        <label>
                          Shared folder on this computer
                          <div className="flex gap-2">
                            <input
                              disabled={locked}
                              readOnly
                              value={connection.local_root}
                            />
                            <button
                              disabled={locked}
                              onClick={() => chooseConnectionPath("local_root")}
                            >
                              Browse
                            </button>
                          </div>
                        </label>
                        <label>
                          Mac access token
                          <input
                            type="password"
                            autoComplete="off"
                            disabled={locked}
                            value={connection.token}
                            onChange={(e) =>
                              setConnection({
                                ...connection,
                                token: e.target.value.trim(),
                              })
                            }
                          />
                        </label>
                        <button disabled={locked} onClick={connectMac}>
                          Connect to Mac
                        </button>
                      </>
                    )}
                  </>
                )}
              </details>
              <label className="repair-option">
                <input
                  type="checkbox"
                  role="switch"
                  disabled={locked}
                  checked={keepBackup}
                  onChange={(e) => setKeepBackup(e.target.checked)}
                />{" "}
                Keep original backups
              </label>
              <label>
                Simultaneous conversions
                <select
                  disabled={locked}
                  value={parallel}
                  onChange={(e) => setParallel(Number(e.target.value))}
                >
                  <option value={1}>1 · default</option>
                  <option value={2}>2 · higher queue throughput</option>
                </select>
              </label>
              <p>
                Two jobs share GPU memory and disk bandwidth. A failed movie
                stays failed while later queued movies continue. Cancel stops
                both.
              </p>
              <p>
                Use FFmpeg with NVENC on Windows or VideoToolbox on Mac.
                Executables beside the app and your system PATH are detected
                automatically.
              </p>
              {(["ffmpeg", "ffprobe"] as const).map((key) => (
                <label key={key}>
                  {key}
                  <div className="flex gap-2">
                    <input
                      value={env[key]}
                      disabled={locked}
                      onChange={(e) =>
                        setEnv({ ...env, [key]: e.target.value })
                      }
                    />
                    <button disabled={locked} onClick={() => chooseTool(key)}>
                      Browse
                    </button>
                  </div>
                </label>
              ))}
              <details onToggle={(event) => {
                const opened = event.currentTarget.open;
                setLicensesOpen(opened);
                if (opened && !licenseText) void import("../THIRD_PARTY_LICENSES.txt?raw").then(m => setLicenseText(m.default)).catch(() => setLicenseText("License notices are also included with the release download."));
              }}>
                <summary>License and third-party notices</summary>
                {licensesOpen && <pre className="license-notices">{appLicense + "\n\n" + (licenseText || "Loading notices…")}</pre>}
              </details>
            </section>
          )}

          {error && (
            <div className="error" role="alert">
              <span>{error}</span>
              <button onClick={() => setError("")} aria-label="Dismiss error">
                <X size={16} />
              </button>
            </div>
          )}

          {!current ? (
            <div className="welcome">
              <span className="hero-icon">
                <Film size={40} strokeWidth={1} />
              </span>
              <p className="eyebrow">MAKE ROOM FOR MORE</p>
              <h1>
                Good movies.
                <br />
                <span>Smaller files.</span>
              </h1>
              <p>
                Choose your movies, keep the tracks you want,
                <br />
                and let your graphics hardware do the work.
              </p>
              <button className="primary" onClick={add} disabled={locked}>
                <Plus size={18} /> Add your first movie
              </button>
              <div className="features">
                <span>NVIDIA NVENC</span>
                <span>APPLE SILICON</span>
                <span>ORIGINAL FRAME TIMING</span>
              </div>
            </div>
          ) : (
            <>
              <div className="title-row">
                <div>
                  <p className="eyebrow">MOVIE SETTINGS</p>
                  <h1>{filename(current.path).replace(/\.[^.]+$/, "")}</h1>
                  <p className="meta">
                    {v?.width} × {v?.height} <span> / </span>{" "}
                    {v?.codec_name.toUpperCase()} <span> / </span>{" "}
                    {v?.r_frame_rate} fps <span> / </span>{" "}
                    {size(Number(current.probe.format.size))}
                  </p>
                </div>
                <button
                  className="quiet"
                  disabled={locked}
                  aria-label="Remove movie"
                  onClick={() => {
                    setMovies((ms) => ms.filter((m) => m.id !== current.id));
                    setSelected(
                      movies.find((m) => m.id !== current.id)?.id || "",
                    );
                  }}
                >
                  <X size={18} />
                </button>
              </div>

              <fieldset disabled={locked}>
                <section>
                  <div className="section-title">
                    <SlidersHorizontal size={17} />
                    <h2>Conversion options</h2>
                  </div>
                  <div className="option-list">
                    <label>
                      <span>
                        Recommended audio outputs
                        <small>
                          AAC for stereo, E-AC-3 for surround; preserve lossless
                          tracks.
                        </small>
                      </span>
                      <input
                        type="checkbox"
                        role="switch"
                        checked={!!current.recommended}
                        onChange={(e) => toggleRecommended(e.target.checked)}
                      />
                    </label>

                    <label>
                      <span>
                        Repair small audio timestamp regressions
                        <small>
                          Repair audio regressions up to 250 ms; larger errors
                          fail this movie.
                        </small>
                      </span>
                      <input
                        type="checkbox"
                        role="switch"
                        checked={current.repairAudioTimestamps}
                        onChange={(e) =>
                          edit({ repairAudioTimestamps: e.target.checked })
                        }
                      />
                    </label>

                    <label>
                      <span>
                        Automatically deinterlace
                        <small>
                          Convert flagged interlaced frames to progressive at
                          the original frame rate.
                        </small>
                      </span>
                      <input
                        type="checkbox"
                        role="switch"
                        checked={current.deinterlace !== false}
                        disabled={current.video === "copy"}
                        onChange={(e) =>
                          edit({ deinterlace: e.target.checked })
                        }
                      />
                    </label>

                    <label>
                      <span>
                        NVIDIA split-frame encoding
                        <small>
                          Use two encoder engines for one movie. Gains vary when
                          running two movies.
                        </small>
                      </span>
                      <input
                        type="checkbox"
                        role="switch"
                        disabled={current.video !== "nvenc"}
                        checked={
                          current.video === "nvenc" && !!current.splitEncode
                        }
                        onChange={(e) =>
                          edit({ splitEncode: e.target.checked })
                        }
                      />
                    </label>
                  </div>
                </section>
                <section>
                  <div className="section-title">
                    <Film size={17} />
                    <h2>Picture</h2>
                    <span>Frame timing always preserved</span>
                  </div>
                  <div className="grid grid-cols-2 gap-4">
                    <label>
                      Video encoder
                      <select
                        value={current.video}
                        onChange={(e) =>
                          edit({
                            video: e.target.value,
                            downscale:
                              e.target.value === "copy"
                                ? false
                                : current.downscale,
                          })
                        }
                      >
                        <option value="nvenc">NVIDIA · HEVC / P7 HQ</option>
                        <option value="apple">Apple Silicon · HEVC</option>
                        <option value="copy">
                          Passthrough · original video
                        </option>
                      </select>
                    </label>
                    <label>
                      Resolution
                      <select
                        value={String(current.downscale)}
                        disabled={current.video === "copy"}
                        onChange={(e) =>
                          edit({ downscale: e.target.value === "true" })
                        }
                      >
                        <option value="false">Keep original resolution</option>
                        <option value="true">
                          Fit within 1920 × 1080 · never upscale
                        </option>
                      </select>
                    </label>
                  </div>
                  {current.video !== "copy" && (
                    <label className="quality">
                      Quality{" "}
                      <input
                        type="range"
                        min="16"
                        max="32"
                        value={current.quality}
                        onChange={(e) =>
                          edit({ quality: Number(e.target.value) })
                        }
                      />
                      <span>
                        {current.quality} ·{" "}
                        {current.quality <= 20
                          ? "Higher quality"
                          : current.quality <= 25
                            ? "Balanced"
                            : "Smaller files"}
                      </span>
                    </label>
                  )}
                  <p className="hint">
                    {current.video === "copy"
                      ? "Video stays bit-for-bit intact. Audio selections still apply."
                      : "Recommended: HEVC, quality 24, original resolution. Lower quality numbers favor detail. Hardware quality scales differ."}
                  </p>
                  {isHdr && current.video !== "copy" && (
                    <p className="warning">
                      HDR / Dolby Vision detected. Choose video passthrough to
                      preserve its metadata.
                    </p>
                  )}
                </section>

                <section>
                  <div className="section-title">
                    <AudioLines size={18} />
                    <h2>Audio tracks</h2>
                    <span>
                      {
                        current.probe.streams.filter(
                          (s) => s.codec_type === "audio",
                        ).length
                      }{" "}
                      available
                    </span>
                  </div>
                  <div className="track-head">
                    <span>KEEP</span>
                    <span>TRACK / LANGUAGE</span>
                    <span>OUTPUT</span>
                  </div>
                  {current.probe.streams
                    .filter((s) => s.codec_type === "audio")
                    .map((s) => (
                      <div className="track" key={s.index}>
                        <input
                          type="checkbox"
                          aria-label={"Keep audio " + s.index}
                          checked={current.audio[s.index] !== "off"}
                          onChange={(e) =>
                            edit({
                              recommended: false,
                              manualAudio: undefined,
                              audio: {
                                ...current.audio,
                                [s.index]: e.target.checked ? "copy" : "off",
                              },
                            })
                          }
                        />
                        <div>
                          <strong>
                            {s.tags?.title ||
                              s.tags?.name ||
                              "Audio track " + s.index}
                          </strong>
                          <small>
                            {s.tags?.language || "Unknown language"} ·{" "}
                            {s.profile || s.codec_name.toUpperCase()} ·{" "}
                            {s.channels} channels
                          </small>
                        </div>
                        <select
                          aria-label={"Output format for audio " + s.index}
                          disabled={current.audio[s.index] === "off"}
                          value={
                            current.audio[s.index] === "off"
                              ? "copy"
                              : current.audio[s.index]
                          }
                          onChange={(e) =>
                            edit({
                              recommended: false,
                              manualAudio: undefined,
                              audio: {
                                ...current.audio,
                                [s.index]: e.target.value,
                              },
                            })
                          }
                        >
                          <option value="copy">Passthrough</option>
                          <option value="aac">AAC · high quality</option>
                          <option value="eac3" disabled={!canEac3(s)}>
                            E-AC-3 · up to 5.1
                          </option>
                        </select>
                      </div>
                    ))}
                  <p className="hint">
                    Recommendations keep efficient codecs, lossless/object audio
                    and tracks above 5.1 unchanged. Other stereo uses AAC 256
                    kb/s; supported surround uses E-AC-3 640 kb/s. Unchecking
                    restores your previous choices; manual edits turn
                    recommendations off. Track names and channel counts are
                    preserved.
                  </p>
                </section>

                <section>
                  <div className="section-title">
                    <Captions size={18} />
                    <h2>Subtitles</h2>
                    <span>Chapters and font attachments kept</span>
                  </div>
                  <div className="subtitle-list">
                    {current.probe.streams
                      .filter((s) => s.codec_type === "subtitle")
                      .map((s) => (
                        <label key={s.index}>
                          <input
                            type="checkbox"
                            checked={current.subtitles.includes(s.index)}
                            onChange={(e) =>
                              edit({
                                subtitles: e.target.checked
                                  ? [...current.subtitles, s.index]
                                  : current.subtitles.filter(
                                      (i) => i !== s.index,
                                    ),
                              })
                            }
                          />
                          {s.tags?.title ||
                            s.tags?.name ||
                            s.tags?.language ||
                            "Subtitle " + s.index}
                          <small>{s.codec_name}</small>
                        </label>
                      ))}
                    {!current.probe.streams.some(
                      (s) => s.codec_type === "subtitle",
                    ) && (
                      <p className="hint">No subtitle tracks in this file.</p>
                    )}
                  </div>
                </section>
              </fieldset>

              {current.message && (
                <div
                  className={
                    current.state === "Complete" ? "success" : "warning"
                  }
                >
                  {current.message}
                </div>
              )}
            </>
          )}
        </main>
      </div>
      <footer>
        <div className="destination">
          <label>
            <ArrowDownToLine size={16} /> Output
          </label>
          <button
            disabled={locked || replace}
            className="folder"
            onClick={chooseFolder}
          >
            <FolderOpen size={16} />
            <span>
              {replace
                ? "Original location"
                : folder || "Alongside original · choose folder"}
            </span>
          </button>
          <label className="replace">
            <input
              type="checkbox"
              checked={replace}
              disabled={locked}
              onChange={(e) => setReplace(e.target.checked)}
            />{" "}
            Replace original
          </label>
        </div>
        <div className="bottom">
          <div className="status">
            {active ? (
              <>
                <div className="progress">
                  <i style={{ width: pct + "%" }} />
                </div>
                <span>
                  {Object.values(jobProgress).filter((p) => p.running).length}{" "}
                  active · {progress?.phase || "Starting"} · {pct.toFixed(0)}% ·{" "}
                  {progress?.speed}
                </span>
              </>
            ) : (
              <span>
                {movies.filter((m) => m.state === "Ready").length} ready{" "}
                <span className="muted">
                  · MKV output · originals protected
                </span>
              </span>
            )}
          </div>
          {active ? (
            <button onClick={cancel}>
              <Square size={15} /> Cancel queue
            </button>
          ) : (
            <button
              className="primary"
              disabled={locked || !movies.some((m) => m.state === "Ready")}
              onClick={run}
            >
              <Play size={16} /> Convert queue
            </button>
          )}
        </div>
      </footer>
      {context && (
        <div
          className="context-menu"
          role="menu"
          style={{ left: context.x, top: context.y }}
        >
          <button
            autoFocus
            role="menuitem"
            disabled={!movies.find((m) => m.id === context.id)?.output}
            onClick={() => reveal(context.id)}
          >
            <FolderOpen size={16} /> Show converted file in folder
          </button>
        </div>
      )}
    </div>
  );
}

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
