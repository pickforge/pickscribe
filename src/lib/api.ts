import { invoke } from "@tauri-apps/api/core";

export type Stage = "idle" | "recording" | "transcribing" | "cleaning" | "pasting";

export interface HistoryEntry {
  id: number;
  created_at: number;
  duration_ms: number;
  raw_text: string;
  cleaned_text: string | null;
  provider: string;
  model: string;
  language: string;
  word_count: number;
}

export interface StatePayload {
  stage: Stage;
  recording_started_ms: number | null;
  message: string | null;
  error: string | null;
  last_entry: HistoryEntry | null;
}

export interface DayStat {
  day: string;
  words: number;
  sessions: number;
}

export interface Metrics {
  sessions: number;
  words: number;
  speaking_ms: number;
  minutes_saved: number;
  typing_wpm: number;
  avg_words_per_session: number;
  longest_session_ms: number;
  days: DayStat[];
}

export interface AppConfig {
  general: {
    sounds: boolean;
    float_button: boolean;
    typing_wpm: number;
    keep_audio: boolean;
    local_only: boolean;
    theme: string;
  };
  stt: {
    model_path: string;
    language: string;
    audio_target: string;
    recorder: string;
  };
  cleanup: {
    provider: string;
    model: string;
    endpoint: string;
    api_key: string;
    temperature: number;
    timeout_secs: number;
    thinking: string;
    instructions: string;
  };
  paste: {
    method: string;
    chord: string;
    delay_ms: number;
    copy_to_clipboard: boolean;
  };
}

export interface DoctorCheck {
  name: string;
  ok: boolean;
  detail: string;
}

export function desktopApiAvailable() {
  return (
    typeof window !== "undefined" &&
    Boolean((window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__)
  );
}

export const api = {
  getState: () => invoke<StatePayload>("get_state"),
  toggleDictation: () => invoke<void>("toggle_dictation"),
  cancelDictation: () => invoke<void>("cancel_dictation"),
  getAppConfig: () => invoke<AppConfig>("get_app_config"),
  updateAppConfig: (config: AppConfig) => invoke<AppConfig>("update_app_config", { config }),
  listHistory: (search = "", limit = 100, offset = 0) =>
    invoke<HistoryEntry[]>("list_history", { search, limit, offset }),
  deleteHistoryEntry: (id: number) => invoke<void>("delete_history_entry", { id }),
  clearHistory: () => invoke<void>("clear_history"),
  getMetrics: () => invoke<Metrics>("get_metrics"),
  runDoctor: () => invoke<DoctorCheck[]>("run_doctor"),
  listModels: () => invoke<string[]>("list_models"),
  listCleanupModels: (config: AppConfig) =>
    invoke<string[]>("list_cleanup_models", { config }),
  showMainWindow: () => invoke<void>("show_main_window"),
  toggleFloatButton: () => invoke<boolean>("toggle_float_button"),
  getSystemTheme: () => invoke<string>("get_system_theme"),
  copyText: (text: string) => invoke<void>("copy_text", { text }),
};

export const EVENT_STATE = "pickscribe://state";
export const EVENT_LEVEL = "pickscribe://level";
export const EVENT_HISTORY = "pickscribe://history";
export const EVENT_CONFIG = "pickscribe://config";

export function formatDuration(ms: number): string {
  const totalSecs = Math.round(ms / 1000);
  const mins = Math.floor(totalSecs / 60);
  const secs = totalSecs % 60;
  if (mins === 0) return `${secs}s`;
  return `${mins}m ${secs.toString().padStart(2, "0")}s`;
}

export function formatMinutes(minutes: number): string {
  if (minutes < 1) return `${Math.round(minutes * 60)}s`;
  if (minutes < 60) return `${Math.round(minutes)}m`;
  const hours = Math.floor(minutes / 60);
  const rest = Math.round(minutes % 60);
  return `${hours}h ${rest.toString().padStart(2, "0")}m`;
}

export function formatTimestamp(unixSecs: number): string {
  return new Date(unixSecs * 1000).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function formatError(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  return JSON.stringify(err);
}
