import "./styles.css";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";
import {
  PlaybackSupervisor,
  type PlaybackPhase,
  type PlaybackStats,
  type PlaybackViewState,
} from "./playback-supervisor";
import {
  activateFavorite,
  addFavoriteSource,
  deactivateFavorite,
  normalizeLiveSource,
  readLastPlayedSource,
  readSourceLibrary,
  removeFavorite,
  updateActiveFavoriteMetadata,
  writeLastPlayedSource,
  writeSourceLibrary,
  type FavoriteSource,
  type NormalizeSourceResult,
} from "./source-library";

const VOLUME_STORAGE_KEY = "simple-live.volume.v1";
const MUTED_STORAGE_KEY = "simple-live.muted.v1";
const VOLUME_COLLAPSE_DELAY_MS = 160;

const app = element<HTMLElement>("app");
const video = element<HTMLVideoElement>("live-video");
const windowDragRegion = element<HTMLElement>("window-drag-region");
const windowMinimize = element<HTMLButtonElement>("window-minimize");
const windowMaximize = element<HTMLButtonElement>("window-maximize");
const windowClose = element<HTMLButtonElement>("window-close");
const settingsTrigger = element<HTMLButtonElement>("settings-trigger");
const settingsPanel = element<HTMLElement>("settings-panel");
const panelTabList = element<HTMLElement>("panel-tabs");
const panelTabs = Array.from(settingsPanel.querySelectorAll<HTMLButtonElement>("[data-panel-section]"));
const panelViews = Array.from(settingsPanel.querySelectorAll<HTMLElement>("[data-panel-view]"));
const sourceForm = element<HTMLFormElement>("source-form");
const sourceInput = element<HTMLInputElement>("source-input");
const sourceCopy = element<HTMLButtonElement>("source-copy");
const copyFeedback = element<HTMLElement>("copy-feedback");
const sourceError = element<HTMLElement>("source-error");
const sourceFavorite = element<HTMLButtonElement>("source-favorite");
const sourceActionStatus = element<HTMLOutputElement>("source-action-status");
const currentRoom = element<HTMLElement>("current-room");
const currentRoomAvatar = element<HTMLImageElement>("current-room-avatar");
const currentRoomAvatarMark = element<HTMLElement>("current-room-avatar-mark");
const currentRoomState = element<HTMLOutputElement>("current-room-state");
const currentRoomTitle = element<HTMLElement>("current-room-title");
const currentRoomAnchor = element<HTMLElement>("current-room-anchor");
const qualitySelect = element<HTMLSelectElement>("quality-select");
const lineSelect = element<HTMLSelectElement>("line-select");
const streamSwitchStatus = element<HTMLOutputElement>("stream-switch-status");
const favoritesList = element<HTMLElement>("favorites-list");
const favoritesEmpty = element<HTMLElement>("favorites-empty");
const favoritesCount = element<HTMLOutputElement>("favorites-count");
const favoritesAdd = element<HTMLButtonElement>("favorites-add");
const favoritesEmptyAdd = element<HTMLButtonElement>("favorites-empty-add");
const playbackStatus = element<HTMLElement>("playback-status");
const statusEyebrow = element<HTMLElement>("status-eyebrow");
const statusTitle = element<HTMLElement>("status-title");
const statusDetail = element<HTMLElement>("status-detail");
const soundUnlock = element<HTMLButtonElement>("sound-unlock");
const volumeControl = element<HTMLElement>("volume-control");
const volumeToggle = element<HTMLButtonElement>("volume-toggle");
const volumeIcon = element<HTMLElement>("volume-icon");
const volumeSpeaker = element<HTMLElement>("volume-speaker");
const volumeSliderShell = element<HTMLElement>("volume-slider-shell");
const volumeSlider = element<HTMLInputElement>("volume-slider");
const volumeOutput = element<HTMLOutputElement>("volume-output");
const statusAvatar = element<HTMLImageElement>("status-avatar");
const streamInspector = element<HTMLElement>("stream-inspector");
const monitorHeading = element<HTMLElement>("monitor-heading");
const monitorStatus = element<HTMLOutputElement>("monitor-status");
const monitorEmpty = element<HTMLElement>("monitor-empty");
const statQuality = element<HTMLElement>("stat-quality");
const statNominalBitrate = element<HTMLElement>("stat-nominal-bitrate");
const statLiveBitrate = element<HTMLElement>("stat-live-bitrate");
const statResolution = element<HTMLElement>("stat-resolution");
const statFrameRate = element<HTMLElement>("stat-frame-rate");
const statBuffer = element<HTMLElement>("stat-buffer");
const statDroppedFrames = element<HTMLElement>("stat-dropped-frames");
const statLine = element<HTMLElement>("stat-line");
const statConnectionHealth = element<HTMLElement>("stat-connection-health");
const bilibiliAccount = element<HTMLElement>("bilibili-account");
const bilibiliAccountStatus = element<HTMLOutputElement>("bilibili-account-status");
const bilibiliAccountAvatar = element<HTMLImageElement>("bilibili-account-avatar");
const bilibiliAccountMark = element<HTMLElement>("bilibili-account-mark");
const bilibiliAccountName = element<HTMLElement>("bilibili-account-name");
const bilibiliAccountDetail = element<HTMLElement>("bilibili-account-detail");
const bilibiliLogin = element<HTMLButtonElement>("bilibili-login");
const bilibiliLogout = element<HTMLButtonElement>("bilibili-logout");
const bilibiliQrPanel = element<HTMLElement>("bilibili-qr-panel");
const bilibiliQrImage = element<HTMLImageElement>("bilibili-qr-image");
const bilibiliQrStatus = element<HTMLElement>("bilibili-qr-status");
const bilibiliQrRefresh = element<HTMLButtonElement>("bilibili-qr-refresh");
const aboutCopyFeedback = element<HTMLOutputElement>("about-copy-feedback");
const aboutCopyButtons = Array.from(settingsPanel.querySelectorAll<HTMLButtonElement>("[data-copy-value]"));
const appWindow = getCurrentWindow();
const coarsePointer = window.matchMedia("(pointer: coarse)");
const windowsPlatform = /Windows/u.test(navigator.userAgent);

document.documentElement.dataset.input = coarsePointer.matches ? "touch" : "pointer";
document.documentElement.dataset.platform = windowsPlatform ? "windows" : "default";

type PanelSection = "settings" | "favorites" | "monitor" | "about";

interface BilibiliAuthStatus {
  status: "guest" | "authenticated" | "expired" | "unavailable";
  authenticated: boolean;
  userName: string;
  avatarUrl: string;
  userId: string;
  detail: string;
}

interface BilibiliQrLogin {
  imageDataUrl: string;
  expiresInSeconds: number;
}

interface BilibiliQrPoll {
  status: "authenticated" | "scanned" | "unscanned" | "expired";
  message: string;
  auth: BilibiliAuthStatus | null;
}

let currentPhase: PlaybackPhase = "idle";
let statusHideTimer: number | undefined;
let settingsOpen = false;
let panelFocusTimer: number | undefined;
let copyFeedbackTimer: number | undefined;
let aboutCopyFeedbackTimer: number | undefined;
let volumeCollapseTimer: number | undefined;
let qrPollTimer: number | undefined;
let qrLoginGeneration = 0;
let lastBilibiliQn: number | null = null;
let lastAudibleVolume = readSavedVolume();
let library = readSourceLibrary(window.localStorage);
let currentSource: Extract<NormalizeSourceResult, { ok: true }> | null = null;
let latestPlaybackStats: PlaybackStats | null = null;
let activePanelSection: PanelSection = "settings";
let sourceActionTimer: number | undefined;

settingsPanel.inert = true;
video.volume = lastAudibleVolume;
video.muted = window.localStorage.getItem(MUTED_STORAGE_KEY) === "true";
renderVolumeControl();

const supervisor = new PlaybackSupervisor(video, renderPlaybackState, renderPlaybackStats);
setPanelSection("settings");
writeSourceLibrary(window.localStorage, library);
renderFavorites();
void refreshBilibiliAuthStatus();
const savedSource = readLastPlayedSource(window.localStorage, library);

if (savedSource) {
  currentSource = savedSource;
  sourceInput.value = savedSource.source;
  library = activateMatchingFavorite(library, savedSource.id);
  persistLibrary();
  renderCurrentRoomPending(savedSource);
  supervisor.start(savedSource.source);
} else {
  renderPlaybackState({
    phase: "idle",
    headline: "等待直播源",
    detail: "使用右上角的设置按钮粘贴直播间链接",
    mutedByPolicy: false,
  });
  window.setTimeout(() => setSettingsOpen(true), 280);
}

settingsTrigger.addEventListener("click", () => setSettingsOpen(!settingsOpen));
panelTabs.forEach((tab, index) => {
  tab.addEventListener("click", () => setPanelSection(panelSectionOf(tab), true));
  tab.addEventListener("keydown", (event) => navigatePanelTabs(event, index));
});
favoritesAdd.addEventListener("click", showSourceSettings);
favoritesEmptyAdd.addEventListener("click", showSourceSettings);
aboutCopyButtons.forEach((button) => {
  button.addEventListener("click", () => void copyAboutLink(button));
});
windowMinimize.addEventListener("click", () => void appWindow.minimize());
windowMaximize.addEventListener("click", () => void appWindow.toggleMaximize());
windowClose.addEventListener("click", () => void appWindow.close());
windowDragRegion.addEventListener("mousedown", (event) => {
  if (coarsePointer.matches) return;
  if (event.button !== 0) return;
  void appWindow.isFullscreen().then((fullscreen) => {
    if (!fullscreen) void appWindow.startDragging();
  });
});
windowDragRegion.addEventListener("dblclick", (event) => {
  if (!windowsPlatform) return;
  event.preventDefault();
  event.stopPropagation();
  void appWindow.toggleMaximize();
});

sourceForm.addEventListener("submit", (event) => {
  event.preventDefault();
  const result = normalizeLiveSource(sourceInput.value);
  if (!result.ok) {
    sourceError.textContent = result.message;
    sourceInput.setAttribute("aria-invalid", "true");
    sourceInput.focus();
    return;
  }

  sourceError.textContent = "";
  sourceInput.removeAttribute("aria-invalid");
  sourceInput.value = result.source;
  playSource(result);
});

sourceInput.addEventListener("input", () => {
  sourceError.textContent = "";
  sourceInput.removeAttribute("aria-invalid");
  sourceActionStatus.textContent = "";
  renderSourceFavoriteState();
});
sourceInput.addEventListener("paste", (event) => {
  const clipboardText = event.clipboardData?.getData("text");
  if (!clipboardText) return;
  event.preventDefault();
  applyPastedSource(clipboardText);
});
sourceInput.addEventListener("keydown", (event) => {
  if (!isPrimaryShortcut(event, "v")) return;
  event.preventDefault();
  void pasteSourceFromClipboard();
});
sourceCopy.addEventListener("click", () => void copySourceLink());
sourceCopy.addEventListener("keydown", (event) => {
  if (event.key !== "Enter" && event.code !== "Space") return;
  event.preventDefault();
  event.stopPropagation();
  void copySourceLink();
});
sourceFavorite.addEventListener("click", addSourceToFavorites);
qualitySelect.addEventListener("change", () => {
  qualitySelect.disabled = true;
  lineSelect.disabled = true;
  streamSwitchStatus.textContent = "正在切换画质…";
  supervisor.selectQuality(Number(qualitySelect.value));
});
lineSelect.addEventListener("change", () => {
  qualitySelect.disabled = true;
  lineSelect.disabled = true;
  streamSwitchStatus.textContent = "正在切换线路…";
  supervisor.selectLine(Number(lineSelect.value));
});
bilibiliLogin.addEventListener("click", () => void startBilibiliQrLogin());
bilibiliQrRefresh.addEventListener("click", () => void startBilibiliQrLogin());
bilibiliLogout.addEventListener("click", () => void logoutBilibili());

volumeToggle.addEventListener("click", () => {
  if (coarsePointer.matches) expandVolumeControl();
  void toggleMute();
});
volumeToggle.addEventListener("pointerenter", expandVolumeControl);
volumeToggle.addEventListener("pointerleave", () => scheduleVolumeCollapse(true));
volumeSliderShell.addEventListener("pointerenter", expandVolumeControl);
volumeSliderShell.addEventListener("pointerleave", () => scheduleVolumeCollapse(true));
volumeControl.addEventListener("keydown", expandVolumeControl);
app.addEventListener("pointerdown", (event) => {
  if (event.target instanceof Node && !volumeControl.contains(event.target)) {
    collapseVolumeControl();
  }
});
document.addEventListener("focusin", () => {
  if (!volumeControl.contains(document.activeElement)) scheduleVolumeCollapse();
});
volumeSlider.addEventListener("input", () => void setVolumeFromSlider());
video.addEventListener("volumechange", renderVolumeControl);
soundUnlock.addEventListener("click", () => void restoreSound());
video.addEventListener("click", () => {
  if (!soundUnlock.hidden) void restoreSound();
});
app.addEventListener("dblclick", (event) => {
  if (coarsePointer.matches) return;
  const target = event.target;
  if (!(target instanceof Element)) return;
  if (target.closest("button, input, select, .settings-panel, .window-drag-region, .window-controls")) return;
  event.preventDefault();
  event.stopPropagation();
  void toggleFullscreen();
}, { capture: true });

document.addEventListener("keydown", (event) => {
  const target = document.activeElement ?? event.target;
  if (isPrimaryShortcut(event, "c")) {
    const selected = selectedText(target);
    if (selected) {
      event.preventDefault();
      void writeText(selected);
      return;
    }
  }
  const isInteractive =
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement ||
    target instanceof HTMLButtonElement ||
    target instanceof HTMLSelectElement ||
    (target instanceof HTMLElement && target.isContentEditable);
  if (event.key === "Escape" && settingsOpen) {
    setSettingsOpen(false);
    return;
  }
  if (isInteractive) return;
  if (event.code === "Space") {
    event.preventDefault();
    void supervisor.togglePause();
  }
  if (event.key.toLowerCase() === "f") {
    event.preventDefault();
    void toggleFullscreen();
  }
});

window.addEventListener("beforeunload", () => {
  cancelBilibiliQrPolling();
  supervisor.destroy();
});

function renderPlaybackState(state: PlaybackViewState): void {
  currentPhase = state.phase;
  app.dataset.phase = state.phase;
  statusEyebrow.textContent = eyebrowFor(state.phase);
  statusTitle.textContent = state.headline;
  statusDetail.textContent = state.detail;
  soundUnlock.hidden = !state.mutedByPolicy;
  if (currentSource && ["resolving", "connecting", "recovering"].includes(state.phase)) {
    currentRoom.hidden = false;
    currentRoomState.dataset.state = "connecting";
    currentRoomState.textContent = state.phase === "recovering" ? "正在恢复" : "正在连接";
    qualitySelect.disabled = true;
    lineSelect.disabled = true;
  }
  window.clearTimeout(statusHideTimer);
  playbackStatus.classList.add("is-visible");

  if ((state.phase === "playing" || state.phase === "replay") && !state.mutedByPolicy) {
    statusHideTimer = window.setTimeout(() => {
      if (currentPhase === "playing" || currentPhase === "replay") {
        playbackStatus.classList.remove("is-visible");
      }
    }, 3_800);
  }
}

function renderPlaybackStats(stats: PlaybackStats): void {
  latestPlaybackStats = stats.active ? stats : null;
  streamInspector.hidden = !stats.active;
  monitorEmpty.hidden = stats.active;
  monitorHeading.dataset.state = stats.active && (stats.live || stats.replay) ? "active" : "idle";
  monitorStatus.textContent = stats.active
    ? stats.live
      ? "直播接收正常"
      : stats.replay
        ? "回放接收正常"
        : "等待开播"
    : "等待播放";
  setAvatar(statusAvatar, stats.avatarUrl);
  if (!stats.active) {
    if (currentSource) renderCurrentRoomPending(currentSource);
    return;
  }

  renderCurrentRoomStats(stats);

  if (stats.platform === "bilibili") {
    const qn = /\bQN\s+(\d+)\b/u.exec(stats.qualityLabel)?.[1];
    lastBilibiliQn = qn ? Number(qn) : null;
  }

  const playing = stats.live || stats.replay;

  statQuality.textContent =
    stats.qualityLabel || (stats.replay ? "回放" : stats.live ? "原画" : "待开播");
  statNominalBitrate.textContent = formatBitrate(stats.nominalBitrateKbps);
  statLiveBitrate.textContent = playing
    ? stats.measuredBitrateKbps === null
      ? "采集中"
      : formatBitrate(stats.measuredBitrateKbps)
    : "—";
  statResolution.textContent = stats.resolution || (playing ? "采集中" : "—");
  statFrameRate.textContent =
    stats.frameRate === null ? (playing ? "采集中" : "—") : `${stats.frameRate.toFixed(1)} fps`;
  statBuffer.textContent =
    stats.bufferedSeconds === null
      ? playing
        ? "采集中"
        : "—"
      : `${stats.bufferedSeconds.toFixed(1)} 秒`;
  statDroppedFrames.textContent = playing
    ? formatDroppedFrames(stats.droppedFrames, stats.totalFrames)
    : "—";
  statLine.textContent = stats.lineLabel || "—";
  statConnectionHealth.textContent = stats.connectionHealth || "—";

  const nextLibrary = updateActiveFavoriteMetadata(library, {
    platform: stats.platform,
    platformLabel: stats.platformLabel,
    roomId: stats.roomId,
    anchor: stats.anchor,
    title: stats.title,
    avatarUrl: stats.avatarUrl,
  });
  if (nextLibrary !== library) {
    library = nextLibrary;
    persistLibrary();
    renderFavorites();
    renderSourceFavoriteState();
  }
}

function renderCurrentRoomPending(source: Extract<NormalizeSourceResult, { ok: true }>): void {
  currentRoom.hidden = false;
  currentRoomState.dataset.state = "connecting";
  currentRoomState.textContent = "正在连接";
  currentRoomTitle.textContent = `${source.platformLabel} 直播间 ${source.roomId}`;
  currentRoomAnchor.textContent = "正在读取标题与主播信息";
  setAvatar(currentRoomAvatar, "");
  currentRoomAvatarMark.hidden = false;
  qualitySelect.disabled = true;
  lineSelect.disabled = true;
  streamSwitchStatus.textContent = "";
}

function renderCurrentRoomStats(stats: PlaybackStats): void {
  currentRoom.hidden = false;
  const state = stats.live ? "live" : stats.replay ? "replay" : "offline";
  currentRoomState.dataset.state = state;
  currentRoomState.textContent = stats.live ? "直播中" : stats.replay ? "回放" : "未开播";
  currentRoomTitle.textContent = stats.title || `${stats.platformLabel} 直播间 ${stats.roomId}`;
  currentRoomAnchor.textContent = stats.anchor
    ? `${stats.anchor} · ${stats.platformLabel}`
    : `${stats.platformLabel} · 房间 ${stats.roomId}`;
  setAvatar(currentRoomAvatar, stats.avatarUrl);
  currentRoomAvatarMark.hidden = Boolean(stats.avatarUrl);

  updateSelectOptions(
    qualitySelect,
    stats.qualityOptions.map((option) => ({ value: option.value, label: option.label })),
    stats.selectedQualityValue,
  );
  updateSelectOptions(
    lineSelect,
    stats.lineOptions.map((option) => ({ value: option.index, label: option.label })),
    stats.selectedLineIndex,
  );
  const selectable = stats.live && !stats.replay;
  qualitySelect.disabled = !selectable || stats.qualityOptions.length <= 1;
  lineSelect.disabled = !selectable || stats.lineOptions.length <= 1;
  streamSwitchStatus.textContent = selectable
    ? [stats.qualityLabel, stats.lineLabel].filter(Boolean).join(" · ")
    : stats.replay
      ? "回放使用平台提供的固定画质与线路"
      : "开播后可以切换画质和线路";
}

function updateSelectOptions(
  select: HTMLSelectElement,
  options: Array<{ value: number; label: string }>,
  selected: number,
): void {
  const signature = options.map((option) => `${option.value}:${option.label}`).join("|");
  if (select.dataset.options !== signature) {
    select.replaceChildren(...options.map((option) => {
      const element = document.createElement("option");
      element.value = String(option.value);
      element.textContent = option.label;
      return element;
    }));
    select.dataset.options = signature;
  }
  select.value = String(selected);
  if (select.selectedIndex < 0 && select.options.length > 0) select.selectedIndex = 0;
}

function renderFavorites(): void {
  favoritesCount.value = String(library.favorites.length);
  favoritesCount.textContent = `${library.favorites.length} 个直播间`;
  favoritesEmpty.hidden = library.favorites.length > 0;
  const rows = library.favorites.map((favorite) => createFavoriteRow(favorite));
  favoritesList.replaceChildren(...rows);
  renderSourceFavoriteState();
}

function playSource(source: Extract<NormalizeSourceResult, { ok: true }>): void {
  currentSource = source;
  latestPlaybackStats = null;
  lastBilibiliQn = null;
  library = activateMatchingFavorite(library, source.id);
  persistLibrary();
  writeLastPlayedSource(window.localStorage, source.source);
  renderFavorites();
  renderCurrentRoomPending(source);
  setSettingsOpen(false);
  supervisor.start(source.source);
}

function addSourceToFavorites(): void {
  const normalized = normalizeLiveSource(sourceInput.value);
  if (!normalized.ok) {
    sourceError.textContent = normalized.message;
    sourceInput.setAttribute("aria-invalid", "true");
    sourceInput.focus();
    return;
  }

  const existed = library.favorites.some((favorite) => favorite.id === normalized.id);
  library = addFavoriteSource(library, normalized);
  if (currentSource?.id === normalized.id) {
    library = activateFavorite(library, normalized.id);
    const stats = latestPlaybackStats;
    if (stats?.active) {
      library = updateActiveFavoriteMetadata(library, {
        platform: stats.platform,
        platformLabel: stats.platformLabel,
        roomId: stats.roomId,
        anchor: stats.anchor,
        title: stats.title,
        avatarUrl: stats.avatarUrl,
      });
    }
  }
  persistLibrary();
  renderFavorites();
  showSourceAction(existed ? "这个直播间已经在收藏中" : "已加入收藏");
}

function activateMatchingFavorite(
  state: typeof library,
  id: string,
): typeof library {
  return state.favorites.some((favorite) => favorite.id === id)
    ? activateFavorite(state, id)
    : deactivateFavorite(state);
}

function renderSourceFavoriteState(): void {
  const normalized = normalizeLiveSource(sourceInput.value);
  const saved = normalized.ok && library.favorites.some((favorite) => favorite.id === normalized.id);
  sourceFavorite.classList.toggle("is-saved", saved);
  sourceFavorite.setAttribute("aria-pressed", String(saved));
  const label = sourceFavorite.querySelector("span");
  if (label) label.textContent = saved ? "已收藏" : "收藏";
  sourceFavorite.title = saved ? "已在收藏列表中" : "加入收藏列表";
}

function showSourceAction(message: string): void {
  window.clearTimeout(sourceActionTimer);
  sourceActionStatus.textContent = message;
  sourceActionTimer = window.setTimeout(() => {
    sourceActionStatus.textContent = "";
    sourceActionTimer = undefined;
  }, 2_200);
}

function createFavoriteRow(favorite: FavoriteSource): HTMLElement {
  const row = document.createElement("div");
  row.className = "favorite-row";
  const active = favorite.id === library.activeId;
  if (active) row.classList.add("is-active");

  const select = document.createElement("button");
  select.type = "button";
  select.className = "favorite-select";
  select.setAttribute("aria-label", `播放 ${favorite.anchor || favorite.platformLabel} 直播间`);
  if (active) select.setAttribute("aria-current", "true");
  select.addEventListener("click", () => selectFavorite(favorite.id));

  const avatar = document.createElement("span");
  avatar.className = "favorite-avatar";
  const avatarFallback = document.createElement("i");
  avatarFallback.className = "bi bi-play-fill";
  avatarFallback.setAttribute("aria-hidden", "true");
  avatar.append(avatarFallback);
  if (favorite.avatarUrl) {
    const image = document.createElement("img");
    image.src = favorite.avatarUrl;
    image.alt = "";
    image.referrerPolicy = "no-referrer";
    image.addEventListener("load", () => avatarFallback.remove(), { once: true });
    image.addEventListener("error", () => image.remove(), { once: true });
    avatar.append(image);
  }

  const copy = document.createElement("span");
  copy.className = "favorite-copy";
  const meta = document.createElement("span");
  meta.className = "favorite-meta";
  const platform = document.createElement("span");
  platform.className = "favorite-platform";
  platform.textContent = favorite.platformLabel;
  meta.append(platform);
  if (active) {
    const activeLabel = document.createElement("span");
    activeLabel.className = "favorite-active-label";
    activeLabel.textContent = "正在播放";
    meta.append(activeLabel);
  }
  const title = document.createElement("span");
  title.className = "favorite-title";
  title.textContent = favorite.anchor || `${favorite.platformLabel} · ${favorite.roomId}`;
  const subtitle = document.createElement("span");
  subtitle.className = "favorite-subtitle";
  subtitle.textContent = favorite.title || `房间 ${favorite.roomId}`;
  copy.append(meta, title, subtitle);
  select.append(avatar, copy);

  const remove = document.createElement("button");
  remove.type = "button";
  remove.className = "favorite-remove";
  remove.setAttribute("aria-label", `移除 ${favorite.anchor || favorite.platformLabel} 收藏`);
  remove.title = "移除收藏";
  const trashIcon = document.createElement("i");
  trashIcon.className = "bi bi-trash3";
  trashIcon.setAttribute("aria-hidden", "true");
  remove.append(trashIcon);
  remove.addEventListener("click", () => deleteFavorite(favorite.id));

  row.append(select, remove);
  return row;
}

function selectFavorite(id: string): void {
  const selected = library.favorites.find((favorite) => favorite.id === id);
  if (!selected) return;
  const normalized = normalizeLiveSource(selected.source);
  if (!normalized.ok) return;
  sourceInput.value = normalized.source;
  sourceError.textContent = "";
  sourceInput.removeAttribute("aria-invalid");
  playSource(normalized);
}

function deleteFavorite(id: string): void {
  library = removeFavorite(library, id);
  persistLibrary();
  renderFavorites();
}

function persistLibrary(): void {
  writeSourceLibrary(window.localStorage, library);
}

async function refreshBilibiliAuthStatus(): Promise<void> {
  bilibiliLogin.disabled = true;
  try {
    const status = await invoke<BilibiliAuthStatus>("get_bilibili_auth_status");
    renderBilibiliAuthStatus(status);
    if (status.authenticated) maybeUpgradeActiveBilibiliAfterAuth();
  } catch (error) {
    renderBilibiliAuthStatus({
      status: "unavailable",
      authenticated: false,
      userName: "",
      avatarUrl: "",
      userId: "",
      detail: `暂时无法读取登录状态：${errorMessage(error)}`,
    });
  } finally {
    bilibiliLogin.disabled = false;
  }
}

function renderBilibiliAuthStatus(status: BilibiliAuthStatus): void {
  bilibiliAccount.dataset.status = status.status;
  bilibiliAccountStatus.textContent = accountStatusLabel(status);
  bilibiliAccountName.textContent = status.authenticated
    ? status.userName || "已保存登录账号"
    : status.status === "expired"
      ? "登录已失效"
      : "未登录";
  bilibiliAccountDetail.textContent = status.detail
    || (status.authenticated
      ? "播放时会按这个账号可用的实际最高档位取流。"
      : "游客仍可播放；扫码后可使用账号开放的更高画质。");
  bilibiliLogin.hidden = status.authenticated;
  bilibiliLogout.hidden = !status.authenticated;
  setAvatar(bilibiliAccountAvatar, status.avatarUrl);
  bilibiliAccountMark.hidden = Boolean(status.avatarUrl);
  bilibiliAccountAvatar.onerror = () => {
    bilibiliAccountAvatar.hidden = true;
    bilibiliAccountMark.hidden = false;
  };
}

function accountStatusLabel(status: BilibiliAuthStatus): string {
  if (status.status === "authenticated") return "已登录";
  if (status.status === "expired") return "已失效";
  if (status.status === "unavailable") return status.authenticated ? "待验证" : "不可用";
  return "游客";
}

async function startBilibiliQrLogin(): Promise<void> {
  const generation = ++qrLoginGeneration;
  window.clearTimeout(qrPollTimer);
  qrPollTimer = undefined;
  bilibiliLogin.disabled = true;
  bilibiliQrRefresh.hidden = true;
  bilibiliQrPanel.hidden = false;
  bilibiliQrPanel.dataset.state = "loading";
  bilibiliQrImage.removeAttribute("src");
  bilibiliQrStatus.textContent = "正在生成安全登录二维码…";

  try {
    const login = await invoke<BilibiliQrLogin>("start_bilibili_qr_login");
    if (generation !== qrLoginGeneration) return;
    bilibiliQrPanel.dataset.state = "waiting";
    bilibiliQrImage.src = login.imageDataUrl;
    bilibiliQrStatus.textContent = `请使用哔哩哔哩客户端扫码 · ${Math.round(login.expiresInSeconds / 60)} 分钟内有效`;
    window.requestAnimationFrame(() => {
      if (generation === qrLoginGeneration) {
        bilibiliQrPanel.scrollIntoView({ behavior: "smooth", block: "center" });
      }
    });
    qrPollTimer = window.setTimeout(() => void pollBilibiliQrLogin(generation), 700);
  } catch (error) {
    if (generation !== qrLoginGeneration) return;
    bilibiliQrPanel.dataset.state = "error";
    bilibiliQrStatus.textContent = `二维码生成失败：${errorMessage(error)}`;
    bilibiliQrRefresh.hidden = false;
  } finally {
    if (generation === qrLoginGeneration) bilibiliLogin.disabled = false;
  }
}

async function pollBilibiliQrLogin(generation: number): Promise<void> {
  if (generation !== qrLoginGeneration) return;
  try {
    const result = await invoke<BilibiliQrPoll>("poll_bilibili_qr_login");
    if (generation !== qrLoginGeneration) return;
    bilibiliQrStatus.textContent = result.message;
    bilibiliQrPanel.dataset.state = result.status;

    if (result.status === "authenticated" && result.auth) {
      renderBilibiliAuthStatus(result.auth);
      bilibiliQrRefresh.hidden = true;
      restartActiveBilibiliSource();
      qrPollTimer = window.setTimeout(() => {
        if (generation === qrLoginGeneration) bilibiliQrPanel.hidden = true;
      }, 1_100);
      return;
    }
    if (result.status === "expired") {
      bilibiliQrRefresh.hidden = false;
      return;
    }
    qrPollTimer = window.setTimeout(() => void pollBilibiliQrLogin(generation), 1_500);
  } catch (error) {
    if (generation !== qrLoginGeneration) return;
    bilibiliQrPanel.dataset.state = "error";
    bilibiliQrStatus.textContent = `扫码状态查询失败：${errorMessage(error)}`;
    bilibiliQrRefresh.hidden = false;
  }
}

async function logoutBilibili(): Promise<void> {
  cancelBilibiliQrPolling();
  bilibiliLogout.disabled = true;
  try {
    const status = await invoke<BilibiliAuthStatus>("logout_bilibili");
    renderBilibiliAuthStatus(status);
    bilibiliQrPanel.hidden = true;
    restartActiveBilibiliSource();
  } catch (error) {
    bilibiliAccountDetail.textContent = `退出登录失败：${errorMessage(error)}`;
  } finally {
    bilibiliLogout.disabled = false;
  }
}

function cancelBilibiliQrPolling(): void {
  qrLoginGeneration += 1;
  window.clearTimeout(qrPollTimer);
  qrPollTimer = undefined;
}

function restartActiveBilibiliSource(): void {
  if (currentSource?.platform === "bilibili") supervisor.start(currentSource.source);
}

function maybeUpgradeActiveBilibiliAfterAuth(attempt = 0): void {
  if (currentSource?.platform !== "bilibili") return;
  if (lastBilibiliQn !== null) {
    if (lastBilibiliQn < 10_000) supervisor.start(currentSource.source);
    return;
  }
  if (attempt >= 4) {
    supervisor.start(currentSource.source);
    return;
  }
  window.setTimeout(() => maybeUpgradeActiveBilibiliAfterAuth(attempt + 1), 500);
}

function expandVolumeControl(): void {
  window.clearTimeout(volumeCollapseTimer);
  volumeCollapseTimer = undefined;
  volumeControl.classList.add("is-expanded");
  volumeSliderShell.inert = false;
  volumeSliderShell.setAttribute("aria-hidden", "false");
  volumeToggle.setAttribute("aria-expanded", "true");
}

function collapseVolumeControl(): void {
  window.clearTimeout(volumeCollapseTimer);
  volumeCollapseTimer = undefined;
  volumeControl.classList.remove("is-expanded");
  volumeSliderShell.inert = true;
  volumeSliderShell.setAttribute("aria-hidden", "true");
  volumeToggle.setAttribute("aria-expanded", "false");
}

function scheduleVolumeCollapse(force = false): void {
  window.clearTimeout(volumeCollapseTimer);
  volumeCollapseTimer = window.setTimeout(() => {
    if (force || !volumeControl.contains(document.activeElement)) {
      collapseVolumeControl();
    }
    volumeCollapseTimer = undefined;
  }, VOLUME_COLLAPSE_DELAY_MS);
}

async function toggleMute(): Promise<void> {
  const currentlySilent = video.muted || video.volume === 0;
  if (currentlySilent && video.volume === 0) {
    video.volume = lastAudibleVolume > 0 ? lastAudibleVolume : 0.8;
  }

  await supervisor.setMuted(!currentlySilent);
  persistAudioPreference();
  renderVolumeControl();
}

async function setVolumeFromSlider(): Promise<void> {
  const volume = Math.min(1, Math.max(0, Number(volumeSlider.value) / 100));
  video.volume = volume;

  if (volume > 0) {
    lastAudibleVolume = volume;
    if (video.muted) await supervisor.setMuted(false);
  } else if (!video.muted) {
    await supervisor.setMuted(true);
  }

  persistAudioPreference();
  renderVolumeControl();
}

async function restoreSound(): Promise<void> {
  if (video.volume === 0) video.volume = lastAudibleVolume > 0 ? lastAudibleVolume : 0.8;
  await supervisor.unlockSound();
  if (!video.muted) persistAudioPreference();
  renderVolumeControl();
}

function renderVolumeControl(): void {
  const silent = video.muted || video.volume === 0;
  const volumePercent = Math.round(video.volume * 100);
  const volumeLevel = silent
    ? "muted"
    : volumePercent <= 33
      ? "low"
      : volumePercent <= 66
        ? "medium"
        : "high";
  volumeSlider.value = String(volumePercent);
  volumeSlider.style.setProperty("--volume-progress", `${silent ? 0 : volumePercent}%`);
  volumeOutput.value = `${silent ? 0 : volumePercent}%`;
  volumeToggle.setAttribute("aria-pressed", String(silent));
  volumeToggle.setAttribute("aria-label", silent ? "取消静音" : "静音");
  volumeToggle.title = silent ? "取消静音" : "静音";
  volumeIcon.dataset.level = volumeLevel;
  volumeSpeaker.classList.toggle("bi-volume-off-fill", !silent);
  volumeSpeaker.classList.toggle("bi-volume-mute-fill", silent);
}

function persistAudioPreference(): void {
  window.localStorage.setItem(VOLUME_STORAGE_KEY, String(lastAudibleVolume));
  window.localStorage.setItem(MUTED_STORAGE_KEY, String(video.muted || video.volume === 0));
}

function readSavedVolume(): number {
  const savedVolume = Number(window.localStorage.getItem(VOLUME_STORAGE_KEY));
  return Number.isFinite(savedVolume) && savedVolume > 0 && savedVolume <= 1 ? savedVolume : 1;
}

function setSettingsOpen(open: boolean): void {
  if (open === settingsOpen) return;

  settingsOpen = open;
  window.clearTimeout(panelFocusTimer);
  panelFocusTimer = undefined;

  settingsPanel.setAttribute("aria-hidden", String(!open));
  settingsPanel.inert = !open;
  settingsTrigger.setAttribute("aria-expanded", String(open));

  if (open) {
    setPanelSection("settings");
    app.classList.add("settings-open");
    panelFocusTimer = window.setTimeout(() => {
      if (coarsePointer.matches) {
        panelFocusTimer = undefined;
        return;
      }
      if (settingsOpen && bilibiliQrPanel.hidden) {
        sourceInput.focus();
      } else if (settingsOpen) {
        bilibiliQrPanel.scrollIntoView({ behavior: "smooth", block: "center" });
      }
      panelFocusTimer = undefined;
    }, 180);
    return;
  }

  app.classList.remove("settings-open");
  supervisor.setStatsInspectionActive(false);
  if (settingsPanel.contains(document.activeElement)) settingsTrigger.focus();
}

function setPanelSection(section: PanelSection, focusTab = false): void {
  const nextIndex = panelTabs.findIndex((tab) => panelSectionOf(tab) === section);
  const previousIndex = panelTabs.findIndex((tab) => panelSectionOf(tab) === activePanelSection);
  activePanelSection = section;
  panelTabList.style.setProperty("--active-tab-index", String(Math.max(0, nextIndex)));
  panelTabList.dataset.direction = nextIndex >= previousIndex ? "forward" : "backward";
  panelTabs.forEach((tab) => {
    const active = panelSectionOf(tab) === section;
    tab.classList.toggle("is-active", active);
    tab.setAttribute("aria-selected", String(active));
    tab.tabIndex = active ? 0 : -1;
    if (active && focusTab) tab.focus();
  });
  panelViews.forEach((view, index) => {
    const active = view.dataset.panelView === section;
    view.classList.toggle("is-active", active);
    view.dataset.panelPosition = active ? "active" : index < nextIndex ? "before" : "after";
    view.setAttribute("aria-hidden", String(!active));
    view.inert = !active;
  });
  supervisor.setStatsInspectionActive(settingsOpen && section === "monitor");
}

function panelSectionOf(tab: HTMLElement): PanelSection {
  const section = tab.dataset.panelSection;
  if (section === "favorites" || section === "monitor" || section === "about") return section;
  return "settings";
}

function navigatePanelTabs(event: KeyboardEvent, currentIndex: number): void {
  if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
  event.preventDefault();
  let nextIndex = currentIndex;
  if (event.key === "Home") nextIndex = 0;
  if (event.key === "End") nextIndex = panelTabs.length - 1;
  if (event.key === "ArrowLeft") nextIndex = (currentIndex - 1 + panelTabs.length) % panelTabs.length;
  if (event.key === "ArrowRight") nextIndex = (currentIndex + 1) % panelTabs.length;
  const nextTab = panelTabs[nextIndex];
  if (nextTab) setPanelSection(panelSectionOf(nextTab), true);
}

function showSourceSettings(): void {
  setPanelSection("settings");
  window.requestAnimationFrame(() => sourceInput.focus());
}

function eyebrowFor(phase: PlaybackPhase): string {
  switch (phase) {
    case "playing":
      return "LIVE";
    case "replay":
      return "REPLAY";
    case "recovering":
    case "connecting":
    case "resolving":
      return "CONNECTING";
    case "offline":
      return "STANDBY";
    case "paused":
      return "PAUSED";
    case "error":
      return "WAITING";
    default:
      return "QILIU";
  }
}

async function toggleFullscreen(): Promise<void> {
  const fullscreen = await appWindow.isFullscreen();
  await appWindow.setFullscreen(!fullscreen);
}

async function copySourceLink(): Promise<void> {
  const source = sourceInput.value.trim();
  if (!source) {
    sourceError.textContent = "请先粘贴直播间链接";
    sourceInput.setAttribute("aria-invalid", "true");
    sourceInput.focus();
    return;
  }

  try {
    await writeText(source);
    window.clearTimeout(copyFeedbackTimer);
    sourceCopy.classList.add("is-copied");
    sourceCopy.setAttribute("aria-label", "直播间链接已复制");
    sourceCopy.title = "已复制";
    copyFeedback.textContent = "直播间链接已复制";
    copyFeedbackTimer = window.setTimeout(() => {
      sourceCopy.classList.remove("is-copied");
      sourceCopy.setAttribute("aria-label", "复制直播间链接");
      sourceCopy.title = "复制链接";
      copyFeedback.textContent = "";
    }, 2_500);
  } catch {
    sourceError.textContent = "复制失败，请使用 ⌘C";
    sourceInput.focus();
    sourceInput.select();
  }
}

async function copyAboutLink(button: HTMLButtonElement): Promise<void> {
  const value = button.dataset.copyValue;
  if (!value) return;
  try {
    await writeText(value);
    window.clearTimeout(aboutCopyFeedbackTimer);
    aboutCopyButtons.forEach((item) => item.classList.toggle("is-copied", item === button));
    aboutCopyFeedback.textContent = "GitHub 链接已复制";
    aboutCopyFeedbackTimer = window.setTimeout(() => {
      button.classList.remove("is-copied");
      aboutCopyFeedback.textContent = "";
    }, 2_200);
  } catch {
    aboutCopyFeedback.textContent = "复制失败，请稍后重试";
  }
}

async function pasteSourceFromClipboard(): Promise<void> {
  try {
    applyPastedSource(await readText());
  } catch {
    sourceError.textContent = "无法读取剪贴板，请检查系统权限后重试";
    sourceInput.setAttribute("aria-invalid", "true");
  }
}

function applyPastedSource(value: string): void {
  const normalized = normalizeLiveSource(value);
  if (!normalized.ok) {
    sourceInput.value = value.trim();
    sourceError.textContent = normalized.message;
    sourceInput.setAttribute("aria-invalid", "true");
    renderSourceFavoriteState();
    return;
  }
  sourceInput.value = normalized.source;
  sourceError.textContent = "";
  sourceInput.removeAttribute("aria-invalid");
  renderSourceFavoriteState();
}

function isPrimaryShortcut(event: KeyboardEvent, key: string): boolean {
  return (event.metaKey || event.ctrlKey)
    && !event.altKey
    && event.key.toLowerCase() === key;
}

function selectedText(target: EventTarget | null): string {
  if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) {
    const start = target.selectionStart ?? 0;
    const end = target.selectionEnd ?? start;
    return target.value.slice(start, end);
  }
  return window.getSelection()?.toString() ?? "";
}

function element<T extends HTMLElement>(id: string): T {
  const found = document.getElementById(id);
  if (!found) throw new Error(`Missing required element: ${id}`);
  return found as T;
}

function setAvatar(image: HTMLImageElement, source: string): void {
  if (!source) {
    image.hidden = true;
    image.removeAttribute("src");
    return;
  }
  if (image.src !== source) image.src = source;
  image.hidden = false;
  image.onerror = () => {
    image.hidden = true;
  };
}

function formatBitrate(kbps: number): string {
  if (!Number.isFinite(kbps) || kbps <= 0) return "—";
  return kbps >= 1_000 ? `${(kbps / 1_000).toFixed(kbps >= 10_000 ? 0 : 1)} Mbps` : `${Math.round(kbps)} kbps`;
}

function formatDroppedFrames(dropped: number | null, total: number | null): string {
  if (dropped === null || total === null) return "采集中";
  if (total <= 0) return `${dropped} 帧`;
  return `${dropped} 帧 · ${((dropped / total) * 100).toFixed(2)}%`;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
