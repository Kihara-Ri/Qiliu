// 更新中心：编排“体验补丁（自动）”与“应用版本更新（需确认）”两条通道。
//
// - 体验补丁：启动即加载已激活的补丁入口；检查（启动后延迟一次、之后每 6 小时、
//   或手动）发现新补丁时自动下载应用，无需用户确认。加载连续失败会由 Rust 端
//   自动停用，用户可在设置里重开或回滚。
// - 版本更新：检查到新版本后在“关于”面板展示，用户确认后桌面端自动下载、
//   验签、安装并重启；Android 打开发布页下载。

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
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
  type PatchCheckOutcome,
  type PatchEntryModule,
  type PatchModuleApi,
  type PatchState,
} from "./update-logic";

const AUTO_SWEEP_DELAY_MS = 12_000;
const AUTO_SWEEP_INTERVAL_MS = 6 * 60 * 60 * 1_000;

const updateSection = document.getElementById("app-update");
const updateStatus = document.getElementById("update-status") as HTMLOutputElement | null;
const updateVersionLine = document.getElementById("update-version-line");
const updateNotes = document.getElementById("update-notes");
const updateFootnote = document.getElementById("update-footnote");
const updateProgress = document.getElementById("update-progress");
const updateProgressBar = document.getElementById("update-progress-bar") as HTMLProgressElement | null;
const updateProgressLabel = document.getElementById("update-progress-label");
const updateCheckButton = document.getElementById("update-check") as HTMLButtonElement | null;
const updateConfirmButton = document.getElementById("update-confirm") as HTMLButtonElement | null;
const updateOpenPageButton = document.getElementById("update-open-page") as HTMLButtonElement | null;
const patchRollbackButton = document.getElementById("patch-rollback") as HTMLButtonElement | null;
const patchAutoToggle = document.getElementById("patch-auto-toggle") as HTMLInputElement | null;

let patchState: PatchState | null = null;
let updateInfo: AppUpdateInfo | null = null;
let checking = false;
let installing = false;
let checkError: string | null = null;
let appliedPatchVersion: number | null = null;
let lastPatchOutcomeText = "";
let sweepDelayTimer: number | undefined;
let sweepInterval: number | undefined;

export function initAppUpdates(): void {
  if (!updateSection || !updateStatus || !updateVersionLine || !updateCheckButton) {
    throw new Error("Missing required update elements");
  }

  void bootstrap();

  updateCheckButton.addEventListener("click", () => void sweep(true));
  updateConfirmButton?.addEventListener("click", () => void confirmUpdateInstall());
  updateOpenPageButton?.addEventListener("click", () => void openReleasePage());
  patchRollbackButton?.addEventListener("click", () => void rollbackPatch());
  patchAutoToggle?.addEventListener("change", () => void setPatchEnabled(patchAutoToggle.checked));

  sweepDelayTimer = window.setTimeout(() => {
    sweepDelayTimer = undefined;
    void sweep(false);
    sweepInterval = window.setInterval(() => void sweep(false), AUTO_SWEEP_INTERVAL_MS);
  }, AUTO_SWEEP_DELAY_MS);
}

export function disposeAppUpdates(): void {
  if (sweepDelayTimer !== undefined) window.clearTimeout(sweepDelayTimer);
  sweepDelayTimer = undefined;
  if (sweepInterval !== undefined) window.clearInterval(sweepInterval);
  sweepInterval = undefined;
}

async function bootstrap(): Promise<void> {
  try {
    patchState = await invoke<PatchState>("get_patch_state");
  } catch {
    patchState = null;
  }
  await applyActivePatch(patchState);
  render();
}

async function sweep(manual: boolean): Promise<void> {
  if (checking || installing) return;
  checking = true;
  if (manual) checkError = null;
  render();
  try {
    const [patchResult, info] = await Promise.all([
      invoke<{ state: PatchState; outcome: PatchCheckOutcome }>("check_patches"),
      invoke<AppUpdateInfo>("check_app_update"),
    ]);
    patchState = patchResult.state;
    updateInfo = info;
    checkError = null;
    lastPatchOutcomeText = describePatchOutcome(patchResult.outcome);
    await applyActivePatch(patchState);
    if (manual && patchResult.outcome.kind === "failed") {
      checkError = patchResult.outcome.reason;
    }
  } catch (error) {
    // 自动检查失败保持安静（避免每 6 小时打扰）；手动检查才显示错误。
    if (manual) checkError = errorMessage(error);
  } finally {
    checking = false;
    render();
  }
}

async function applyActivePatch(state: PatchState | null): Promise<void> {
  if (!state || !state.enabled || state.autoDisabled) return;
  if (state.activeVersion == null || !state.entryUrl) return;
  if (appliedPatchVersion === state.activeVersion) return;
  try {
    const module = (await import(/* @vite-ignore */ state.entryUrl)) as Partial<PatchEntryModule>;
    const apply = module?.default;
    if (typeof apply === "function") {
      await apply(patchApi(state));
    }
    appliedPatchVersion = state.activeVersion;
    await invoke("report_patch_health", { ok: true });
  } catch (error) {
    try {
      await invoke("report_patch_health", {
        ok: false,
        message: errorMessage(error).slice(0, 300),
      });
    } catch {
      // 上报失败不打断主流程。
    }
  }
}

function patchApi(state: PatchState): PatchModuleApi {
  return {
    version: state.activeVersion ?? 0,
    notes: state.notes,
    log: (...arguments_: unknown[]) => console.log("[patch]", ...arguments_),
  };
}

async function confirmUpdateInstall(): Promise<void> {
  if (!updateInfo?.available || installing) return;
  if (!updateInfo.canAutoInstall) {
    await openReleasePage();
    return;
  }
  installing = true;
  render();
  const unlisten = await listen<{ downloaded: number; total: number | null; finished: boolean }>(
    "app-update://progress",
    event => renderProgress(event.payload),
  );
  try {
    // 桌面端安装完成后应用会自动重启，通常不会走到这里。
    await invoke("install_app_update");
  } catch (error) {
    checkError = errorMessage(error);
  } finally {
    unlisten();
    installing = false;
    render();
  }
}

async function openReleasePage(): Promise<void> {
  const url = updateInfo?.releaseUrl ?? "https://github.com/Kihara-Ri/Qiliu/releases/latest";
  try {
    await invoke("open_release_page", { url });
  } catch (error) {
    checkError = errorMessage(error);
    render();
  }
}

async function rollbackPatch(): Promise<void> {
  try {
    patchState = await invoke<PatchState>("rollback_patch");
    appliedPatchVersion = null;
  } catch (error) {
    checkError = errorMessage(error);
  }
  render();
}

async function setPatchEnabled(enabled: boolean): Promise<void> {
  try {
    patchState = await invoke<PatchState>("set_patch_enabled", { enabled });
    if (enabled) {
      appliedPatchVersion = null;
      await applyActivePatch(patchState);
    }
  } catch (error) {
    checkError = errorMessage(error);
  }
  render();
}

function render(): void {
  if (!updateSection || !updateStatus || !updateVersionLine) return;

  if (patchAutoToggle) patchAutoToggle.checked = Boolean(patchState?.enabled);

  const status = describeUpdateStatus(updateInfo, { checking, installing, error: checkError });
  updateStatus.textContent = status.label;
  updateSection.dataset.tone = status.tone;

  const lastCheck = formatCheckTime(patchState?.lastCheck ?? null);
  updateVersionLine.textContent = updateInfo
    ? `${describeVersionLine(updateInfo, patchState)}${lastCheck ? ` · 检查于 ${lastCheck}` : ""}`
    : describeVersionLine({ currentVersion: "…" }, patchState);

  const notes = updateInfo?.available ? summarizeNotes(updateInfo.notes) : "";
  if (updateNotes) {
    updateNotes.textContent = notes;
    updateNotes.hidden = !notes;
  }

  if (updateFootnote) {
    const footnote = describePatchFootnote(patchState) || lastPatchOutcomeText;
    updateFootnote.textContent = footnote;
    updateFootnote.hidden = !footnote;
  }

  if (updateCheckButton) updateCheckButton.disabled = checking || installing;
  if (updateConfirmButton) {
    updateConfirmButton.hidden = !(updateInfo?.available && updateInfo.canAutoInstall) || installing;
    updateConfirmButton.textContent = installing ? "正在更新…" : "下载并重启";
    updateConfirmButton.disabled = installing;
  }
  if (updateOpenPageButton) {
    updateOpenPageButton.hidden = !(updateInfo?.available && !updateInfo.canAutoInstall);
  }
  if (patchRollbackButton) {
    patchRollbackButton.hidden = !canRollbackPatch(patchState);
  }

  if (updateProgress && !installing) updateProgress.hidden = true;
}

function renderProgress(payload: { downloaded: number; total: number | null; finished: boolean }): void {
  if (!updateProgress || !updateProgressBar || !updateProgressLabel) return;
  updateProgress.hidden = payload.finished;
  const percent = progressPercent(payload.downloaded, payload.total);
  if (percent == null) {
    updateProgressBar.removeAttribute("value");
    updateProgressLabel.textContent = formatBytes(payload.downloaded);
    return;
  }
  updateProgressBar.value = percent;
  updateProgressBar.max = 100;
  updateProgressLabel.textContent = `${percent}%（${formatBytes(payload.downloaded)} / ${formatBytes(payload.total ?? 0)}）`;
}

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 MB";
  const mb = bytes / (1024 * 1024);
  return mb >= 10 ? `${Math.round(mb)} MB` : `${mb.toFixed(1)} MB`;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
