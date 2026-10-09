// Development-only UI preview with mocked Tauri IPC (never part of the app bundle).
// Open http://localhost:1420/preview.html?view=dashboard|new|recording|lecture|settings|models
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import React from "react";
import ReactDOM from "react-dom/client";
import App from "../App";
import "../styles.css";

mockWindows("main");
(window as any).__TAURI_INTERNALS__.convertFileSrc = (p: string) => p;
const view = new URLSearchParams(location.search).get("view") ?? "dashboard";
const dark = new URLSearchParams(location.search).get("dark") === "1";

const settings = {
  lectures_root: "/Users/student/Documents/LectureCapture", known_roots: [],
  detector: { sample_fps: 2, analysis_width: 320, pixel_threshold: 28, shift_tolerance: 1, block_size: 16, block_change_ratio: 0.03, cursor_max_blocks: 2, max_small_components: 3, stable_frames: 3, max_unstable_ms: 15000, min_change_interval_ms: 800, dedupe_hash_distance: 10, reveal_mode: "merge", ignore_blank: true, masks: [], motion_filter: true, motion_hold_ms: 4000, live_video_fraction: 0.6 },
  audio: { capture_system: true, capture_microphone: false, microphone_device: null, loopback_device: null, only_application: null, microphone_gain: 1, opus_bitrate: 32000, retention: "keep", silence_threshold_db: -60, silence_warn_ms: 30000, vad: {} },
  transcription: { enabled: true, model: "base", language: "pl", threads: 4, live: true, beam_size: 1, initial_prompt: null },
  output: { webp_archive: false, png_compression: "balanced" }, consent_acknowledged: view !== "consent",
  capture_shortcut: "CmdOrCtrl+Shift+S", keep_awake: true, theme: dark ? "dark" : "light", last_target: null, last_crop: null,
  last_subject: "Algorytmy i struktury danych", auto_leave_meeting: true,
};
const slide = (i: number) => `/dev-preview/00${i}.png`;
const lectures = [
  { path: "/L/1", title: "Drzewa binarne", subject: "Algorytmy i struktury danych", folder: "2026-10-09_Algorytmy", started_at: "2026-10-09T10:15:00+02:00", duration_ms: 5_412_000, slides: 42, status: "completed", transcription: "completed", model: "base", thumbnail: slide(1), size_bytes: 98_000_000, has_audio: true },
  { path: "/L/2", title: "Całki niewłaściwe", subject: "Analiza matematyczna II", folder: "x", started_at: "2026-10-08T12:00:00+02:00", duration_ms: 5_300_000, slides: 31, status: "completed", transcription: "running", model: "small", thumbnail: slide(2), size_bytes: 71_000_000, has_audio: true },
  { path: "/L/3", title: "Systemy operacyjne – wstęp", subject: null, folder: "x", started_at: "2026-10-07T08:00:00+02:00", duration_ms: 4_100_000, slides: 27, status: "recovered", transcription: "partial", model: "base", thumbnail: slide(3), size_bytes: 55_000_000, has_audio: true },
];
const models = [
  { info: { id: "tiny", label: "Whisper Tiny (multilingual)", description: "Najszybszy, najmniej dokładny. ~75 MB.", file: "", size: 77691713, sha256: "", default: false }, installed: true, verified: true, path: "" },
  { info: { id: "base", label: "Whisper Base (multilingual)", description: "Domyślny – dobry kompromis szybkości i jakości. ~148 MB.", file: "", size: 147951465, sha256: "", default: true }, installed: true, verified: true, path: "" },
  { info: { id: "small", label: "Whisper Small (multilingual)", description: "Dokładniejszy, wolniejszy. ~488 MB.", file: "", size: 487601967, sha256: "", default: false }, installed: false, verified: false, path: null },
  { info: { id: "large-v3-turbo-q5", label: "Whisper Large v3 Turbo (q5_0)", description: "Najlepsza jakość dla języka polskiego. ~574 MB.", file: "", size: 574041195, sha256: "", default: false }, installed: false, verified: false, path: null },
];
const words = (t0: number, text: string) => text.split(" ").map((w, i) => ({ s: t0 + i * 400, e: t0 + i * 400 + 350, w }));
const parts = (t0: number, text: string, prev = false, next = false) => [{ start_ms: t0, end_ms: t0 + 5000, words: words(t0, text), continues_from_prev: prev, continues_to_next: next }];
const status = {
  state: "recording", title: "Algorytmy i struktury danych", lecture_dir: "/Users/student/Documents/LectureCapture/2026-10-09_Algorytmy-i-struktury-danych",
  elapsed_ms: 2_745_000, slides: 17, occurrences: 21, last_slide_id: 17, last_slide_path: slide(2), current_slide_id: 17,
  video: { state: "ok", detail: null, frames: 3120, idle_ticks: 2200, dropped_frames: 0, last_frame_ms: 2_744_000, width: 2560, height: 1440 },
  audio: { level_db: -23, peak: 0.3, sources: [{ kind: "system", enabled: true, received_samples: 1, last_push_ms: 1, level_db: -23, resyncs: 0 }], silent_for_ms: 0, no_signal: false, recorded_ms: 2_744_000, lost: null, speech_ratio: 0.71 },
  transcription: { state: "running", model: "base", queue_len: 1, done: 96, failed: 0, lag_ms: 9000, error: null, progress: null, speed: 31.5, last_text: "" },
  lecture_bytes: 61_000_000, free_bytes: 212_000_000_000,
  warnings: [{ t_ms: 1_200_000, level: "info", message: "Ten slajd jest już zapisany (#9)." }],
  recent_segments: [
    { start_ms: 2_701_000, text: "Złożoność wyszukiwania w zrównoważonym drzewie wynosi logarytm z n." },
    { start_ms: 2_712_000, text: "Zwróćcie uwagę, że w najgorszym przypadku drzewo może zdegenerować się do listy." },
    { start_ms: 2_730_000, text: "Dlatego stosujemy drzewa AVL albo czerwono-czarne, o których powiemy za chwilę." },
  ],
};
const detail = {
  path: "/L/1", subject: "Algorytmy i struktury danych", size_bytes: 98_000_000, has_audio: true, prompt_exists: true, pending_chunks: 0, failed_chunks: 0,
  slide_paths: { "1": slide(1), "2": slide(2), "3": slide(3) },
  records: [
    { chunk_id: 1, start_ms: 2000, end_ms: 9000, text: "Dzień dobry państwu. Dzisiaj omówimy drzewa binarne.", words: [], lang: "pl", no_speech_prob: 0, model: "base" },
    { chunk_id: 2, start_ms: 65000, end_ms: 80000, text: "Drzewo binarne to struktura danych, w której każdy węzeł ma co najwyżej dwoje dzieci.", words: [], lang: "pl", no_speech_prob: 0, model: "base" },
  ],
  assignment: {
    before_first: [], outside: [],
    slides: [
      { occurrence_id: "occ-0001", slide_id: 1, start_ms: 0, end_ms: 60000, parts: parts(2000, "Dzień dobry państwu. Dzisiaj omówimy drzewa binarne i ich zastosowania w praktyce.", false, true) },
      { occurrence_id: "occ-0002", slide_id: 2, start_ms: 60000, end_ms: 240000, parts: [...parts(60500, "i ich implementację.", true, false), ...parts(65000, "Drzewo binarne to struktura danych, w której każdy węzeł ma co najwyżej dwoje dzieci. Wysokość drzewa decyduje o złożoności operacji.")] },
      { occurrence_id: "occ-0003", slide_id: 3, start_ms: 240000, end_ms: 400000, parts: parts(241000, "Przejdźmy do przykładu wstawiania elementu do drzewa.") },
      { occurrence_id: "occ-0004", slide_id: 1, start_ms: 400000, end_ms: 460000, parts: parts(401000, "Wróćmy na chwilę do definicji.") },
    ],
  },
  manifest: {
    schema: "lecturecapture/manifest", schema_version: 1,
    lecture: { id: "x", title: "Algorytmy i struktury danych", folder: "2026-10-09_Algorytmy", started_at: "2026-10-09T10:15:00+02:00", ended_at: null, duration_ms: 5_412_000, last_alive_ms: 0, status: "completed", notes: [] },
    capture: { source: { kind: "window", id: "1", title: "Spotkanie | Microsoft Teams", app_name: "Microsoft Teams", width: 2560, height: 1440 }, crop: { x: 0, y: 0.1, w: 0.75, h: 0.8 }, image_format: "png" },
    audio: { file: "audio/recording.ogg", bitrate: 32000, retention: "keep", duration_ms: 5_412_000, sources: ["system"] },
    transcription: { enabled: true, engine: "whisper.cpp 1.8.2", model: "base", language: "pl", status: "completed", chunks_total: 210, chunks_done: 210, chunks_failed: 0, error: null, detected_languages: ["pl"] },
    slides: [1, 2, 3].map((i) => ({ id: i, file: `slides/00${i}.png`, archive_file: null, sha256: "abc" + i, dhash: "", width: 1920, height: 1080, bytes: 412000, captured_at: "", captured_ms: 0, trigger: i === 3 ? "manual" : "auto", build_of: null, updates: i === 2 ? 2 : 0, occurrences: i === 1 ? ["a", "b"] : ["c"], display_start_ms: 0, display_end_ms: 0 })),
    timeline: [], gaps: [{ kind: "paused", start_ms: 1_800_000, end_ms: 1_860_000, detail: null }],
  },
};

mockIPC((cmd) => {
  switch (cmd) {
    case "get_settings": return settings;
    case "save_settings": return null;
    case "system_info": return { platform: "macos", arch: "aarch64", version: "0.1.0", whisper: "METAL = 1", models_dir: "~/Library/Application Support/app.lecturecapture/models", free_bytes: 212_000_000_000, lectures_root: settings.lectures_root, recording: view === "recording" };
    case "recording_status": return view === "recording" ? status : null;
    case "take_recoveries": return [];
    case "list_lectures": return lectures;
    case "models_status": return models;
    case "download_status": return view === "models" ? { small: { state: "running", downloaded: 201_000_000, total: 487_601_967, error: null } } : {};
    case "permissions": return { screen: view !== "noperm", microphone: "unknown" };
    case "list_sources": return { displays: [{ id: 1, name: "Monitor 1 (2560×1664)", width: 2560, height: 1664, scale: 2 }], windows: [
      { id: 11, title: "Algorytmy – wykład 3 | Microsoft Teams", app_name: "Microsoft Teams", bundle_id: "com.microsoft.teams2", pid: 1, width: 1400, height: 900, on_screen: true },
      { id: 12, title: "Notatki", app_name: "Notes", bundle_id: "com.apple.Notes", pid: 2, width: 900, height: 700, on_screen: true },
      { id: 13, title: "Dokumentacja – Safari", app_name: "Safari", bundle_id: "com.apple.Safari", pid: 3, width: 1200, height: 800, on_screen: false } ] };
    case "list_audio_devices": return { inputs: [{ name: "Mikrofon MacBook Air", is_default: true }], outputs: [], system_audio_builtin: true };
    case "snapshot": return { width: 2800, height: 1800, data_url: slide(1) };
    case "disk_free": return 212_000_000_000;
    case "get_lecture": return detail;
    case "transcription_status": return { service: null, job: null };
    case "list_subjects": return [
      { name: "Algorytmy i struktury danych", path: "/L/A", lectures: 1 },
      { name: "Analiza matematyczna II", path: "/L/B", lectures: 1 },
      { name: "Fizyka", path: "/L/C", lectures: 0 },
    ];
    case "get_auto_stop": return { schedule: view === "recording" ? { at: "2026-10-09T21:00:00+02:00", leave_meeting: true, window: { app: "com.microsoft.teams2", title: null } } : null, shortcut: "⌘⇧H", can_send_keys: true };
    default: return null;
  }
});

if (view !== "dashboard") {
  const routes: Record<string, unknown> = { new: { name: "new" }, recording: { name: "recording" }, lecture: { name: "lecture", path: "/L/1" }, settings: { name: "settings" }, models: { name: "models" }, consent: { name: "dashboard" }, noperm: { name: "new" } };
  (window as any).__LC_INITIAL_ROUTE__ = routes[view];
}

ReactDOM.createRoot(document.getElementById("root")!).render(<React.StrictMode><App /></React.StrictMode>);
