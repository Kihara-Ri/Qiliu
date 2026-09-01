export const SOURCE_LIBRARY_STORAGE_KEY = "simple-live.source-library.v1";
export const LEGACY_HUYA_SOURCE_STORAGE_KEY = "simple-live.huya-source.v1";

const SOURCE_LIBRARY_VERSION = 1;
const MAX_FAVORITES = 50;

export type LivePlatform = "huya" | "bilibili";

export interface FavoriteSource {
  id: string;
  source: string;
  platform: LivePlatform;
  platformLabel: string;
  roomId: string;
  anchor: string;
  title: string;
  avatarUrl: string;
  addedAt: number;
  lastPlayedAt: number;
}

export interface SourceLibraryState {
  version: 1;
  activeId: string | null;
  favorites: FavoriteSource[];
}

export type NormalizeSourceResult =
  | {
      ok: true;
      source: string;
      id: string;
      roomId: string;
      platform: LivePlatform;
      platformLabel: string;
    }
  | { ok: false; message: string };

export interface FavoriteMetadata {
  platform?: string;
  platformLabel?: string;
  roomId?: string;
  anchor?: string;
  title?: string;
  avatarUrl?: string;
}

export function emptySourceLibrary(): SourceLibraryState {
  return { version: SOURCE_LIBRARY_VERSION, activeId: null, favorites: [] };
}

export function readSourceLibrary(storage: Pick<Storage, "getItem">): SourceLibraryState {
  const saved = storage.getItem(SOURCE_LIBRARY_STORAGE_KEY);
  if (saved) {
    try {
      return sanitizeLibrary(JSON.parse(saved));
    } catch {
      // Fall through to the recoverable legacy source instead of losing it.
    }
  }

  const legacy = storage.getItem(LEGACY_HUYA_SOURCE_STORAGE_KEY)?.trim() ?? "";
  const normalized = normalizeLiveSource(legacy);
  return normalized.ok
    ? addOrActivateSource(emptySourceLibrary(), normalized, Date.now())
    : emptySourceLibrary();
}

export function writeSourceLibrary(
  storage: Pick<Storage, "setItem">,
  library: SourceLibraryState,
): void {
  storage.setItem(SOURCE_LIBRARY_STORAGE_KEY, JSON.stringify(sanitizeLibrary(library)));
}

export function normalizeLiveSource(value: string): NormalizeSourceResult {
  const cleaned = value.replace(/[\u200B-\u200D\u2060\uFEFF]/gu, "").trim();
  if (!cleaned) return { ok: false, message: "请输入虎牙或 Bilibili 直播间链接" };

  let failure: NormalizeSourceResult = {
    ok: false,
    message: "粘贴内容中没有可识别的虎牙或 Bilibili 直播间链接",
  };
  for (const candidate of liveSourceCandidates(cleaned)) {
    const normalized = normalizeLiveSourceCandidate(candidate);
    if (normalized.ok) return normalized;
    failure = normalized;
  }
  return failure;
}

function normalizeLiveSourceCandidate(candidate: string): NormalizeSourceResult {
  const trimmed = stripLinkPunctuation(candidate);

  try {
    const url = new URL(trimmed.includes("://") ? trimmed : `https://${trimmed}`);
    if (url.protocol !== "http:" && url.protocol !== "https:") {
      return { ok: false, message: "直播间链接只支持 http 或 https" };
    }
    if (url.username || url.password) {
      return { ok: false, message: "直播间链接不能包含账号信息" };
    }

    const host = url.hostname.toLowerCase();
    if (host === "huya.com" || host.endsWith(".huya.com")) {
      const roomId = url.pathname.split("/").filter(Boolean)[0] ?? "";
      if (!/^[a-zA-Z0-9_-]{1,64}$/.test(roomId)) {
        return { ok: false, message: "链接中没有有效的虎牙房间号" };
      }
      return {
        ok: true,
        source: `https://www.huya.com/${roomId}`,
        id: `huya:${roomId.toLowerCase()}`,
        roomId,
        platform: "huya",
        platformLabel: "虎牙",
      };
    }

    if (host === "live.bilibili.com" || host.endsWith(".live.bilibili.com")) {
      const roomId = url.pathname.split("/").filter(Boolean)[0] ?? "";
      return normalizeBilibiliRoom(roomId);
    }
    if (host === "bilibili.com" || host.endsWith(".bilibili.com")) {
      return normalizeBilibiliRoom(url.searchParams.get("cid") ?? "");
    }
    return { ok: false, message: "目前只支持虎牙和 Bilibili 直播间链接" };
  } catch {
    return { ok: false, message: "直播间链接格式不正确" };
  }
}

function liveSourceCandidates(value: string): string[] {
  const candidates = [value];
  const pattern = /(?:^|[^\w.-])((?:https?:\/\/)?(?:[a-z0-9-]+\.)*(?:huya\.com|bilibili\.com)(?:[/?#][^\s<>"'`，。；！？]*)?)/giu;
  for (const match of value.matchAll(pattern)) {
    if (match[1]) candidates.push(match[1]);
  }
  return [...new Set(candidates.map(stripLinkPunctuation).filter(Boolean))];
}

function stripLinkPunctuation(value: string): string {
  return value
    .trim()
    .replace(/^[<([{（【「『《]+/u, "")
    .replace(/[>)}\]）】」』》,.;!?，。；！？：、]+$/u, "");
}

export function addOrActivateSource(
  library: SourceLibraryState,
  normalized: Extract<NormalizeSourceResult, { ok: true }>,
  now = Date.now(),
): SourceLibraryState {
  const existing = library.favorites.find((favorite) => favorite.id === normalized.id);
  const favorite: FavoriteSource = existing
    ? { ...existing, source: normalized.source, lastPlayedAt: now }
    : {
        id: normalized.id,
        source: normalized.source,
        platform: normalized.platform,
        platformLabel: normalized.platformLabel,
        roomId: normalized.roomId,
        anchor: "",
        title: "",
        avatarUrl: "",
        addedAt: now,
        lastPlayedAt: now,
      };
  const favorites = [
    favorite,
    ...library.favorites.filter((item) => item.id !== favorite.id),
  ].slice(0, MAX_FAVORITES);
  return { version: SOURCE_LIBRARY_VERSION, activeId: favorite.id, favorites };
}

export function activateFavorite(
  library: SourceLibraryState,
  id: string,
  now = Date.now(),
): SourceLibraryState {
  const favorite = library.favorites.find((item) => item.id === id);
  if (!favorite) return library;
  const activated = { ...favorite, lastPlayedAt: now };
  return {
    version: SOURCE_LIBRARY_VERSION,
    activeId: id,
    favorites: [activated, ...library.favorites.filter((item) => item.id !== id)],
  };
}

export function removeFavorite(library: SourceLibraryState, id: string): SourceLibraryState {
  const favorites = library.favorites.filter((favorite) => favorite.id !== id);
  const activeId = library.activeId === id
    ? favorites[0]?.id ?? null
    : favorites.some((favorite) => favorite.id === library.activeId)
      ? library.activeId
      : favorites[0]?.id ?? null;
  return { version: SOURCE_LIBRARY_VERSION, activeId, favorites };
}

export function updateActiveFavoriteMetadata(
  library: SourceLibraryState,
  metadata: FavoriteMetadata,
): SourceLibraryState {
  if (!library.activeId) return library;
  let changed = false;
  const favorites = library.favorites.map((favorite) => {
    if (favorite.id !== library.activeId) return favorite;
    const next = {
      ...favorite,
      roomId: safeMetadata(metadata.roomId) || favorite.roomId,
      anchor: safeMetadata(metadata.anchor),
      title: safeMetadata(metadata.title),
      avatarUrl: safeMetadata(metadata.avatarUrl),
      platformLabel: safeMetadata(metadata.platformLabel) || favorite.platformLabel,
    };
    const platform = metadata.platform === "huya" || metadata.platform === "bilibili"
      ? metadata.platform
      : favorite.platform;
    next.platform = platform;
    changed = favorite.roomId !== next.roomId
      || favorite.anchor !== next.anchor
      || favorite.title !== next.title
      || favorite.avatarUrl !== next.avatarUrl
      || favorite.platform !== next.platform
      || favorite.platformLabel !== next.platformLabel;
    return changed ? next : favorite;
  });
  return changed ? { ...library, favorites } : library;
}

export function activeFavorite(library: SourceLibraryState): FavoriteSource | undefined {
  return library.favorites.find((favorite) => favorite.id === library.activeId);
}

function normalizeBilibiliRoom(roomId: string): NormalizeSourceResult {
  if (!/^\d{1,20}$/.test(roomId)) {
    return { ok: false, message: "链接中没有有效的 Bilibili 直播间号" };
  }
  return {
    ok: true,
    source: `https://live.bilibili.com/${roomId}`,
    id: `bilibili:${roomId}`,
    roomId,
    platform: "bilibili",
    platformLabel: "Bilibili",
  };
}

function sanitizeLibrary(value: unknown): SourceLibraryState {
  if (!isRecord(value) || !Array.isArray(value.favorites)) return emptySourceLibrary();
  const favorites: FavoriteSource[] = [];
  const seen = new Set<string>();
  for (const raw of value.favorites.slice(0, MAX_FAVORITES)) {
    if (!isRecord(raw)) continue;
    const normalized = normalizeLiveSource(typeof raw.source === "string" ? raw.source : "");
    if (!normalized.ok || seen.has(normalized.id)) continue;
    seen.add(normalized.id);
    favorites.push({
      id: normalized.id,
      source: normalized.source,
      platform: normalized.platform,
      platformLabel: normalized.platformLabel,
      roomId: safeMetadata(raw.roomId) || normalized.roomId,
      anchor: safeMetadata(raw.anchor),
      title: safeMetadata(raw.title),
      avatarUrl: safeMetadata(raw.avatarUrl),
      addedAt: safeTimestamp(raw.addedAt),
      lastPlayedAt: safeTimestamp(raw.lastPlayedAt),
    });
  }
  const requestedActiveId = typeof value.activeId === "string" ? value.activeId : null;
  const activeId = favorites.some((favorite) => favorite.id === requestedActiveId)
    ? requestedActiveId
    : favorites[0]?.id ?? null;
  return { version: SOURCE_LIBRARY_VERSION, activeId, favorites };
}

function safeMetadata(value: unknown): string {
  return typeof value === "string" ? value.trim().slice(0, 1_024) : "";
}

function safeTimestamp(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? value : Date.now();
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}
