import type { ExportFormat, FileJobState, HistoryEntry } from "./api";

export const MEDIA_EXTENSIONS = [
  "wav",
  "mp3",
  "m4a",
  "aac",
  "ogg",
  "opus",
  "flac",
  "wma",
  "mp4",
  "mkv",
  "mov",
  "webm",
  "avi",
  "m4v",
] as const;

export function basename(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  const cut = trimmed.split(/[\\/]/).pop() ?? "";
  return cut || path;
}

function extension(path: string): string {
  const name = basename(path);
  const dot = name.lastIndexOf(".");
  if (dot <= 0) return "";
  return name.slice(dot + 1).toLowerCase();
}

export function isMediaPath(path: string): boolean {
  return (MEDIA_EXTENSIONS as readonly string[]).includes(extension(path));
}

export function pickMediaPaths(paths: string[]): string[] {
  return paths.filter(isMediaPath);
}

export function fileStageLabel(state: FileJobState): string {
  switch (state.stage) {
    case "converting":
      return "Converting…";
    case "transcribing":
      return state.progress > 0 ? `Transcribing ${Math.round(state.progress)}%` : "Transcribing…";
    case "cleaning":
      return "Cleaning up…";
    case "done":
      return "Done";
    case "error":
      return "Failed";
    case "cancelled":
      return "Cancelled";
  }
}

export function isDeterminate(state: FileJobState): boolean {
  return state.stage === "transcribing" && state.progress > 0;
}

export function exportFormats(entry: HistoryEntry): { format: ExportFormat; enabled: boolean }[] {
  const timestamped = Boolean(entry.segments_json);
  return [
    { format: "txt", enabled: true },
    { format: "srt", enabled: timestamped },
    { format: "vtt", enabled: timestamped },
  ];
}
