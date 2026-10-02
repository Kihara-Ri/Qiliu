import { describe, expect, it } from "vitest";
import {
  canRollbackPatch,
  describePatchFootnote,
  describePatchOutcome,
  describeUpdateStatus,
  describeVersionLine,
  formatCheckTime,
  progressPercent,
  summarizeNotes,
  type AppUpdateInfo,
  type PatchState,
} from "./update-logic";

function patchState(overrides: Partial<PatchState> = {}): PatchState {
  return {
    enabled: true,
    autoDisabled: false,
    activeVersion: null,
    notes: "",
    failures: 0,
    lastCheck: null,
    lastError: null,
    entryUrl: null,
    ...overrides,
  };
}

function updateInfo(overrides: Partial<AppUpdateInfo> = {}): AppUpdateInfo {
  return {
    channel: "desktop",
    currentVersion: "1.5.0",
    available: false,
    version: null,
    notes: null,
    pubDate: null,
    canAutoInstall: false,
    releaseUrl: null,
    ...overrides,
  };
}

describe("describeVersionLine", () => {
  it("shows the active patch version when applied", () => {
    expect(describeVersionLine(updateInfo(), patchState({ activeVersion: 3 }))).toBe(
      "应用 1.5.0 · 体验补丁 v3",
    );
  });

  it("shows a placeholder when no patch is active", () => {
    expect(describeVersionLine(updateInfo({ currentVersion: "1.6.0" }), patchState())).toBe(
      "应用 1.6.0 · 体验补丁 —",
    );
  });
});

describe("describePatchOutcome", () => {
  it("describes each outcome kind", () => {
    expect(describePatchOutcome({ kind: "applied", sequence: 4 })).toBe("已自动应用体验补丁 v4");
    expect(describePatchOutcome({ kind: "upToDate" })).toBe("体验补丁已是最新");
    expect(describePatchOutcome({ kind: "skipped" })).toBe("体验补丁已停用，跳过检查");
    expect(describePatchOutcome({ kind: "failed", reason: "网络不可用" })).toBe(
      "体验补丁检查失败：网络不可用",
    );
    expect(describePatchOutcome({ kind: "inapplicable", reason: "版本过低" })).toBe(
      "暂无适用补丁（版本过低）",
    );
  });
});

describe("describeUpdateStatus", () => {
  it("prioritizes installing and checking states", () => {
    expect(describeUpdateStatus(updateInfo({ available: true, version: "2.0.0" }), {
      checking: true,
      installing: true,
      error: null,
    })).toEqual({ label: "正在安装更新…", tone: "busy" });
    expect(
      describeUpdateStatus(null, { checking: true, installing: false, error: null }).tone,
    ).toBe("busy");
  });

  it("marks available, latest, error and idle states", () => {
    expect(describeUpdateStatus(updateInfo({ available: true, version: "1.6.0" }), {
      checking: false,
      installing: false,
      error: null,
    })).toEqual({ label: "新版本 1.6.0 可用", tone: "available" });
    expect(describeUpdateStatus(updateInfo(), {
      checking: false,
      installing: false,
      error: null,
    })).toEqual({ label: "已是最新（1.5.0）", tone: "latest" });
    expect(
      describeUpdateStatus(null, { checking: false, installing: false, error: "超时" }).tone,
    ).toBe("error");
    expect(
      describeUpdateStatus(null, { checking: false, installing: false, error: null }).label,
    ).toBe("尚未检查");
  });
});

describe("summarizeNotes", () => {
  it("drops blank lines and caps line count and width", () => {
    const notes = ["第一行", "", "第二行", "x".repeat(150), ...Array.from({ length: 20 }, (_, i) => `行${i}`)].join("\n");
    const summarized = summarizeNotes(notes, 4, 40);
    expect(summarized.split("\n").length).toBe(4);
  });

  it("returns empty string for null", () => {
    expect(summarizeNotes(null)).toBe("");
  });
});

describe("formatCheckTime", () => {
  it("formats unix seconds as local MM-DD HH:mm", () => {
    const date = new Date(2026, 9, 2, 8, 5);
    expect(formatCheckTime(Math.floor(date.getTime() / 1_000))).toBe("10-02 08:05");
  });

  it("returns empty for null or invalid values", () => {
    expect(formatCheckTime(null)).toBe("");
    expect(formatCheckTime(Number.NaN)).toBe("");
    expect(formatCheckTime(0)).toBe("");
  });
});

describe("canRollbackPatch / describePatchFootnote", () => {
  it("only allows rollback with an active patch", () => {
    expect(canRollbackPatch(patchState({ activeVersion: 2 }))).toBe(true);
    expect(canRollbackPatch(patchState())).toBe(false);
    expect(canRollbackPatch(null)).toBe(false);
  });

  it("explains auto-disable and manual disable first", () => {
    expect(describePatchFootnote(patchState({ autoDisabled: true, lastError: "x" }))).toContain(
      "自动停用",
    );
    expect(describePatchFootnote(patchState({ enabled: false }))).toBe("体验补丁已停用。");
    expect(describePatchFootnote(patchState({ activeVersion: 3, notes: "修复收藏排序" }))).toBe(
      "补丁说明：修复收藏排序",
    );
  });
});

describe("progressPercent", () => {
  it("computes a capped percentage", () => {
    expect(progressPercent(50, 200)).toBe(25);
    expect(progressPercent(300, 200)).toBe(100);
  });

  it("returns null without a positive total", () => {
    expect(progressPercent(50, null)).toBeNull();
    expect(progressPercent(50, 0)).toBeNull();
  });
});
