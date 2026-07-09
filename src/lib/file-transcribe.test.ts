import { describe, expect, it } from "vitest";

import type { FileJobState, HistoryEntry } from "./api";
import {
  basename,
  exportFormats,
  fileStageLabel,
  isDeterminate,
  isMediaPath,
  MEDIA_EXTENSIONS,
  pickMediaPaths,
} from "./file-transcribe";

describe("basename", () => {
  it("extracts the final path segment across separators", () => {
    expect(basename("/home/me/meeting.mp4")).toBe("meeting.mp4");
    expect(basename("C:\\Users\\me\\clip.wav")).toBe("clip.wav");
    expect(basename("bare.mp3")).toBe("bare.mp3");
  });

  it("tolerates trailing separators and empty input", () => {
    expect(basename("/home/me/folder/")).toBe("folder");
    expect(basename("")).toBe("");
  });
});

describe("isMediaPath", () => {
  it("accepts every supported extension case-insensitively", () => {
    for (const ext of MEDIA_EXTENSIONS) {
      expect(isMediaPath(`/tmp/sample.${ext}`)).toBe(true);
      expect(isMediaPath(`/tmp/sample.${ext.toUpperCase()}`)).toBe(true);
    }
  });

  it("rejects non-media and extensionless paths", () => {
    expect(isMediaPath("/tmp/notes.txt")).toBe(false);
    expect(isMediaPath("/tmp/archive.zip")).toBe(false);
    expect(isMediaPath("/tmp/README")).toBe(false);
    expect(isMediaPath("/tmp/.hidden")).toBe(false);
  });
});

describe("pickMediaPaths", () => {
  it("keeps only media paths preserving order", () => {
    expect(
      pickMediaPaths(["/a/one.txt", "/a/two.mp3", "/a/notes.pdf", "/a/four.mov"])
    ).toEqual(["/a/two.mp3", "/a/four.mov"]);
    expect(pickMediaPaths([])).toEqual([]);
  });
});

function jobState(partial: Partial<FileJobState>): FileJobState {
  return {
    stage: "converting",
    progress: 0,
    source_file: "/a/clip.mp4",
    error: null,
    entry_id: null,
    ...partial,
  };
}

describe("fileStageLabel", () => {
  it("renders a label per stage with rounded progress", () => {
    expect(fileStageLabel(jobState({ stage: "converting" }))).toBe("Converting…");
    expect(fileStageLabel(jobState({ stage: "transcribing", progress: 41.6 }))).toBe(
      "Transcribing 42%"
    );
    expect(fileStageLabel(jobState({ stage: "cleaning" }))).toBe("Cleaning up…");
    expect(fileStageLabel(jobState({ stage: "done" }))).toBe("Done");
    expect(fileStageLabel(jobState({ stage: "error" }))).toBe("Failed");
    expect(fileStageLabel(jobState({ stage: "cancelled" }))).toBe("Cancelled");
  });
});

describe("isDeterminate", () => {
  it("is determinate only while transcribing with real progress", () => {
    expect(isDeterminate(jobState({ stage: "transcribing", progress: 1 }))).toBe(true);
    expect(isDeterminate(jobState({ stage: "transcribing", progress: 0 }))).toBe(false);
    expect(isDeterminate(jobState({ stage: "converting", progress: 50 }))).toBe(false);
    expect(isDeterminate(jobState({ stage: "cleaning", progress: 50 }))).toBe(false);
  });
});

function historyEntry(partial: Partial<HistoryEntry>): HistoryEntry {
  return {
    id: 1,
    created_at: 0,
    duration_ms: 0,
    raw_text: "raw",
    cleaned_text: null,
    provider: "none",
    model: "",
    language: "en",
    source_file: null,
    segments_json: null,
    word_count: 1,
    ...partial,
  };
}

describe("exportFormats", () => {
  it("enables srt/vtt only when segments are present", () => {
    expect(exportFormats(historyEntry({ segments_json: null }))).toEqual([
      { format: "txt", enabled: true },
      { format: "srt", enabled: false },
      { format: "vtt", enabled: false },
    ]);
    expect(exportFormats(historyEntry({ segments_json: "[]" }))).toEqual([
      { format: "txt", enabled: true },
      { format: "srt", enabled: true },
      { format: "vtt", enabled: true },
    ]);
  });
});
