// Typed wrappers around the Tauri commands (src-tauri/src/lib.rs).
import { invoke, convertFileSrc } from "@tauri-apps/api/core";

export type CaptureTarget =
  | { kind: "display"; id: number }
  | { kind: "window"; id: number; bundle_id: string | null; title: string | null };

export interface NormRect { x: number; y: number; w: number; h: number }

export interface DisplayInfo { id: number; name: string; width: number; height: number; scale: number }
export interface WindowInfo {
  id: number; title: string; app_name: string; bundle_id: string; pid: number;
  width: number; height: number; on_screen: boolean;
}
export interface SourceList { displays: DisplayInfo[]; windows: WindowInfo[] }
export interface AudioDeviceInfo { name: string; is_default: boolean }
export interface AudioDevices { inputs: AudioDeviceInfo[]; outputs: AudioDeviceInfo[]; system_audio_builtin: boolean }
export interface Permissions { screen: boolean; microphone: string }

export interface DetectorConfig {
  sample_fps: number; analysis_width: number; pixel_threshold: number; shift_tolerance: number;
  block_size: number; block_change_ratio: number; cursor_max_blocks: number; max_small_components: number;
  stable_frames: number; max_unstable_ms: number; min_change_interval_ms: number; dedupe_hash_distance: number;
  reveal_mode: "merge" | "separate"; ignore_blank: boolean; masks: NormRect[];
}
export interface VadConfig {
  frame_ms: number; threshold_above_floor_db: number; absolute_threshold_db: number; min_speech_ms: number;
  hangover_ms: number; preroll_ms: number; max_chunk_ms: number; min_chunk_ms: number; overlap_ms: number; cut_search_ms: number;
}
export interface AudioConfig {
  capture_system: boolean; capture_microphone: boolean; microphone_device: string | null;
  loopback_device: string | null; only_application: string | null; microphone_gain: number;
  opus_bitrate: number; retention: "keep" | "delete_after_transcription"; silence_threshold_db: number;
  silence_warn_ms: number; vad: VadConfig;
}
export interface TranscriptionConfig {
  enabled: boolean; model: string; language: string; threads: number; live: boolean; beam_size: number;
  initial_prompt: string | null;
}
export interface OutputConfig { webp_archive: boolean; png_compression: "fast" | "balanced" | "best" }
export interface Settings {
  lectures_root: string; known_roots: string[]; detector: DetectorConfig; audio: AudioConfig;
  transcription: TranscriptionConfig; output: OutputConfig; consent_acknowledged: boolean;
  capture_shortcut: string; keep_awake: boolean; theme: "system" | "light" | "dark";
  last_target: CaptureTarget | null; last_crop: NormRect | null;
}

export interface SystemInfo {
  platform: string; arch: string; version: string; whisper: string; models_dir: string;
  free_bytes: number | null; lectures_root: string; recording: boolean;
}

export interface ModelInfo { id: string; label: string; description: string; file: string; size: number; sha256: string; default: boolean }
export interface ModelStatus { info: ModelInfo; installed: boolean; verified: boolean; path: string | null }
export interface DownloadStatus { state: string; downloaded: number; total: number; error: string | null }

export interface SourceStats { kind: "system" | "microphone"; enabled: boolean; received_samples: number; last_push_ms: number | null; level_db: number; resyncs: number }
export interface TranscriptionView {
  state: string; model: string; queue_len: number; done: number; failed: number; lag_ms: number;
  error: string | null; progress: number | null; speed: number; last_text: string | null;
}
export interface RecorderStatus {
  state: "recording" | "paused" | "stopping" | "finished";
  title: string; lecture_dir: string; elapsed_ms: number; slides: number; occurrences: number;
  last_slide_id: number | null; last_slide_path: string | null; current_slide_id: number | null;
  video: { state: string; detail: string | null; frames: number; idle_ticks: number; dropped_frames: number; last_frame_ms: number | null; width: number; height: number };
  audio: { level_db: number; peak: number; sources: SourceStats[]; silent_for_ms: number; no_signal: boolean; recorded_ms: number; lost: string | null; speech_ratio: number };
  transcription: TranscriptionView;
  lecture_bytes: number; free_bytes: number | null;
  warnings: { t_ms: number; level: "info" | "warning" | "error"; message: string }[];
  recent_segments: { start_ms: number; text: string }[];
}

export type LectureStatus = "recording" | "completed" | "recovered";
export type TranscriptionStatus = "disabled" | "pending" | "running" | "completed" | "partial" | "failed";

export interface LectureSummary {
  path: string; title: string; folder: string; started_at: string; duration_ms: number; slides: number;
  status: LectureStatus; transcription: TranscriptionStatus; model: string; thumbnail: string | null;
  size_bytes: number; has_audio: boolean;
}

export interface Word { s: number; e: number; w: string }
export interface SegmentRecord { chunk_id: number; start_ms: number; end_ms: number; text: string; words: Word[]; lang: string | null; no_speech_prob: number; model: string }
export interface Part { start_ms: number; end_ms: number; words: Word[]; continues_from_prev: boolean; continues_to_next: boolean }
export interface SlideSpeech { occurrence_id: string; slide_id: number; start_ms: number; end_ms: number; parts: Part[] }
export interface Assignment { slides: SlideSpeech[]; before_first: Part[]; outside: Part[] }

export interface Slide {
  id: number; file: string; archive_file: string | null; sha256: string; dhash: string; width: number; height: number;
  bytes: number; captured_at: string; captured_ms: number; trigger: string; build_of: number | null; updates: number;
  occurrences: string[]; display_start_ms: number | null; display_end_ms: number | null;
}
export interface Occurrence { id: string; slide_id: number; start_ms: number; end_ms: number | null; start_at: string; end_at: string | null }
export interface Gap { kind: string; start_ms: number; end_ms: number | null; detail: string | null }
export interface Manifest {
  schema: string; schema_version: number;
  lecture: { id: string; title: string; folder: string; started_at: string; ended_at: string | null; duration_ms: number | null; last_alive_ms: number; status: LectureStatus; notes: string[] };
  capture: { source: { kind: string; id: string | null; title: string | null; app_name: string | null; width: number | null; height: number | null }; crop: NormRect | null; image_format: string };
  audio: { file: string | null; bitrate: number; retention: string; duration_ms: number | null; sources: string[] };
  transcription: { enabled: boolean; engine: string; model: string; language: string; status: TranscriptionStatus; chunks_total: number; chunks_done: number; chunks_failed: number; error: string | null; detected_languages: string[] };
  slides: Slide[]; timeline: Occurrence[]; gaps: Gap[];
}
export interface LectureDetail {
  path: string; manifest: Manifest; slide_paths: Record<string, string>; records: SegmentRecord[];
  assignment: Assignment; pending_chunks: number; failed_chunks: number; size_bytes: number; has_audio: boolean; prompt_exists: boolean;
}
export interface JobStatus { state: string; progress: number; error: string | null; last_text: string | null; kind: string }
export interface TranscriptionInfo { service: TranscriptionView | null; job: JobStatus | null }
export interface RecoveryReport { folder: string; end_ms: number; missing_slides_removed: number[]; orphan_slides_added: number[]; pending_chunks: number }

export interface AudioSelection {
  capture_system: boolean; capture_microphone: boolean; microphone_device: string | null;
  loopback_device: string | null; only_application: string | null;
}
export interface AudioTestSource { kind: string; description: string; started: boolean; error: string | null; buffers: number; seconds_received: number; level_db: number; peak: number }
export interface StartRequest {
  title: string; output_dir: string | null; target: CaptureTarget; crop: NormRect | null; audio: AudioSelection;
  transcription: boolean; model: string; language: string; live: boolean;
}

export const api = {
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
  systemInfo: () => invoke<SystemInfo>("system_info"),
  diskFree: (path: string) => invoke<number | null>("disk_free", { path }),
  permissions: () => invoke<Permissions>("permissions"),
  requestScreenPermission: () => invoke<boolean>("request_screen_permission"),
  listSources: () => invoke<SourceList>("list_sources"),
  listAudioDevices: () => invoke<AudioDevices>("list_audio_devices"),
  snapshot: (target: CaptureTarget) => invoke<{ width: number; height: number; data_url: string }>("snapshot", { target }),
  audioTest: (selection: AudioSelection, seconds = 3) => invoke<AudioTestSource[]>("audio_test", { selection, seconds }),
  modelsStatus: () => invoke<ModelStatus[]>("models_status"),
  downloadModel: (id: string) => invoke<void>("download_model", { id }),
  downloadStatus: () => invoke<Record<string, DownloadStatus>>("download_status"),
  cancelDownload: (id: string) => invoke<void>("cancel_download", { id }),
  deleteModel: (id: string) => invoke<void>("delete_model", { id }),
  startRecording: (req: StartRequest) => invoke<RecorderStatus>("start_recording", { req }),
  recordingStatus: () => invoke<RecorderStatus | null>("recording_status"),
  pause: () => invoke<void>("pause_recording"),
  resume: () => invoke<void>("resume_recording"),
  captureSlide: () => invoke<void>("capture_slide"),
  setMicrophoneEnabled: (enabled: boolean) => invoke<void>("set_microphone_enabled", { enabled }),
  stopRecording: () => invoke<string>("stop_recording"),
  listLectures: () => invoke<LectureSummary[]>("list_lectures"),
  getLecture: (path: string) => invoke<LectureDetail>("get_lecture", { path }),
  deleteSlide: (path: string, slideId: number) => invoke<void>("delete_slide", { path, slideId }),
  setOccurrenceStart: (path: string, occurrenceId: string, startMs: number) =>
    invoke<void>("set_occurrence_start", { path, occurrenceId, startMs }),
  renameLecture: (path: string, title: string) => invoke<void>("rename_lecture", { path, title }),
  readPrompt: (path: string) => invoke<string>("read_prompt", { path }),
  regenerateDocuments: (path: string) => invoke<void>("regenerate_documents", { path }),
  exportZip: (path: string, dest: string, includeAudio: boolean) => invoke<number>("export_zip", { path, dest, includeAudio }),
  transcriptionStatus: (path: string) => invoke<TranscriptionInfo>("transcription_status", { path }),
  retranscribe: (path: string, model: string, language: string) => invoke<void>("retranscribe", { path, model, language }),
  cancelTranscription: (path: string) => invoke<void>("cancel_transcription", { path }),
  resumeTranscription: (path: string, model?: string) => invoke<void>("resume_transcription", { path, model: model ?? null }),
  takeRecoveries: () => invoke<RecoveryReport[]>("take_recoveries"),
};

export const fileSrc = (path: string, version?: string | number) =>
  convertFileSrc(path) + (version !== undefined ? `?v=${encodeURIComponent(String(version))}` : "");

export const LANGUAGES = [
  { id: "pl", label: "Polski" },
  { id: "en", label: "Angielski" },
  { id: "auto", label: "Wykryj automatycznie" },
];
