// 更新面板的纯展示逻辑：与 Rust 端 patch_hub / app_update 的返回结构一一对应，
// 不做任何 IO，便于单元测试。

/** Rust `PatchStateView`（serde camelCase）。 */
export interface PatchState {
  enabled: boolean;
  autoDisabled: boolean;
  activeVersion: number | null;
  notes: string;
  failures: number;
  lastCheck: number | null;
  lastError: string | null;
  entryUrl: string | null;
}

/** Rust `CheckOutcome`（内部 tag `kind`）。 */
export type PatchCheckOutcome =
  | { kind: "upToDate" }
  | { kind: "applied"; sequence: number }
  | { kind: "inapplicable"; reason: string }
  | { kind: "skipped" }
  | { kind: "failed"; reason: string };

/** Rust `AppUpdateInfo`（serde camelCase）。 */
export interface AppUpdateInfo {
  channel: string;
  currentVersion: string;
  available: boolean;
  version: string | null;
  notes: string | null;
  pubDate: string | null;
  canAutoInstall: boolean;
  releaseUrl: string | null;
}

export interface PatchModuleApi {
  version: number;
  notes: string;
  log: (...arguments_: unknown[]) => void;
}

/** 补丁入口模块约定：默认导出 apply(api)，可省略（纯副作用模块）。 */
export interface PatchEntryModule {
  default?: (api: PatchModuleApi) => void | Promise<void>;
}

/** “应用 1.5.0 · 体验补丁 v3” 这样的版本行。 */
export function describeVersionLine(info: Pick<AppUpdateInfo, "currentVersion">, patch: PatchState | null): string {
  const patchPart = patch?.activeVersion != null
    ? `体验补丁 v${patch.activeVersion}`
    : "体验补丁 —";
  return `应用 ${info.currentVersion} · ${patchPart}`;
}

/** 补丁检查结果的一行说明。 */
export function describePatchOutcome(outcome: PatchCheckOutcome): string {
  switch (outcome.kind) {
    case "applied":
      return `已自动应用体验补丁 v${outcome.sequence}`;
    case "upToDate":
      return "体验补丁已是最新";
    case "inapplicable":
      return `暂无适用补丁（${outcome.reason}）`;
    case "skipped":
      return "体验补丁已停用，跳过检查";
    case "failed":
      return `体验补丁检查失败：${outcome.reason}`;
  }
}

/** 更新状态徽标文案与色调。 */
export function describeUpdateStatus(
  info: AppUpdateInfo | null,
  options: { checking: boolean; installing: boolean; error: string | null },
): { label: string; tone: "idle" | "busy" | "available" | "latest" | "error" } {
  if (options.installing) return { label: "正在安装更新…", tone: "busy" };
  if (options.checking) return { label: "正在检查更新…", tone: "busy" };
  if (options.error) return { label: "检查失败", tone: "error" };
  if (info?.available && info.version) {
    return { label: `新版本 ${info.version} 可用`, tone: "available" };
  }
  if (info && !info.available) {
    return { label: `已是最新（${info.currentVersion}）`, tone: "latest" };
  }
  return { label: "尚未检查", tone: "idle" };
}

/** 发布说明截断：最多 maxLines 行、每行 maxColumns 字符。 */
export function summarizeNotes(notes: string | null, maxLines = 10, maxColumns = 120): string {
  if (!notes) return "";
  const lines = notes
    .replace(/\r\n/g, "\n")
    .split("\n")
    .map(line => line.trim())
    .filter(line => line.length > 0)
    .slice(0, maxLines)
    .map(line => (line.length > maxColumns ? `${line.slice(0, maxColumns - 1)}…` : line));
  return lines.join("\n");
}

/** Unix 秒 → “10-02 14:32” 本地展示；null 返回空串。 */
export function formatCheckTime(unixSeconds: number | null): string {
  if (unixSeconds == null || !Number.isFinite(unixSeconds) || unixSeconds <= 0) return "";
  const date = new Date(unixSeconds * 1_000);
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

/** 是否应显示“回滚补丁”入口：有激活补丁且未被自动停用时才有得回。 */
export function canRollbackPatch(patch: PatchState | null): boolean {
  return Boolean(patch?.activeVersion != null);
}

/** 补丁行的附加说明（停用/自动停用/最近错误）。 */
export function describePatchFootnote(patch: PatchState | null): string {
  if (!patch) return "";
  if (patch.autoDisabled) return "补丁连续加载失败，已自动停用；可重新开启或保持关闭。";
  if (!patch.enabled) return "体验补丁已停用。";
  if (patch.lastError) return patch.lastError;
  if (patch.activeVersion != null && patch.notes) return `补丁说明：${patch.notes}`;
  return "";
}

/** 下载进度百分比；总量未知时返回 null。 */
export function progressPercent(downloaded: number, total: number | null): number | null {
  if (total == null || total <= 0) return null;
  return Math.min(100, Math.floor((downloaded / total) * 100));
}
