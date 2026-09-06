import { describe, expect, it } from "vitest";
import {
  LEGACY_HUYA_SOURCE_STORAGE_KEY,
  LAST_PLAYED_SOURCE_STORAGE_KEY,
  SOURCE_LIBRARY_STORAGE_KEY,
  activateFavorite,
  activeFavorite,
  addFavoriteSource,
  addOrActivateSource,
  deactivateFavorite,
  emptySourceLibrary,
  normalizeLiveSource,
  readSourceLibrary,
  readLastPlayedSource,
  removeFavorite,
  updateActiveFavoriteMetadata,
  writeSourceLibrary,
  writeLastPlayedSource,
} from "./source-library";

function memoryStorage(initial: Record<string, string> = {}): Storage {
  const values = new Map(Object.entries(initial));
  return {
    get length() {
      return values.size;
    },
    clear: () => values.clear(),
    getItem: (key) => values.get(key) ?? null,
    key: (index) => [...values.keys()][index] ?? null,
    removeItem: (key) => values.delete(key),
    setItem: (key, value) => values.set(key, value),
  };
}

describe("live source normalization", () => {
  it("canonicalizes Huya and Bilibili room links", () => {
    expect(normalizeLiveSource("huya.com/196645?from=home")).toMatchObject({
      ok: true,
      source: "https://www.huya.com/196645",
      platform: "huya",
    });
    expect(normalizeLiveSource("https://live.bilibili.com/5050?live_from=81001")).toMatchObject({
      ok: true,
      source: "https://live.bilibili.com/5050",
      platform: "bilibili",
    });
    expect(
      normalizeLiveSource(
        "https://www.bilibili.com/blackboard/live/live-activity-player.html?cid=5050",
      ),
    ).toMatchObject({ ok: true, source: "https://live.bilibili.com/5050" });
  });

  it("extracts and cleans a supported link from copied share text", () => {
    expect(
      normalizeLiveSource(
        "直播链接：【https://live.bilibili.com/22907643?live_from=81001&spm_id_from=333.337.0.0】 复制后打开",
      ),
    ).toMatchObject({
      ok: true,
      source: "https://live.bilibili.com/22907643",
      roomId: "22907643",
    });
    expect(normalizeLiveSource("\u200B访问 huya.com/196645?from=share。"))
      .toMatchObject({ ok: true, source: "https://www.huya.com/196645" });
    expect(
      normalizeLiveSource(
        "复制这段文字后打开应用\nhttps://live.bilibili.com/22907643#/?live_from=84001\n主播正在直播",
      ),
    ).toMatchObject({ ok: true, source: "https://live.bilibili.com/22907643" });
  });

  it("rejects unsupported and malformed links", () => {
    expect(normalizeLiveSource("https://example.com/5050").ok).toBe(false);
    expect(normalizeLiveSource("https://live.bilibili.com/not-a-room").ok).toBe(false);
    expect(normalizeLiveSource("").ok).toBe(false);
  });
});

describe("persistent source library", () => {
  it("migrates the previous single Huya source without losing it", () => {
    const storage = memoryStorage({
      [LEGACY_HUYA_SOURCE_STORAGE_KEY]: "https://www.huya.com/196645",
    });
    const library = readSourceLibrary(storage);
    expect(library.favorites).toHaveLength(1);
    expect(activeFavorite(library)?.source).toBe("https://www.huya.com/196645");
  });

  it("deduplicates favorites and promotes the active source", () => {
    const huya = normalizeLiveSource("https://huya.com/196645");
    const bilibili = normalizeLiveSource("https://live.bilibili.com/5050");
    if (!huya.ok || !bilibili.ok) throw new Error("test source should normalize");

    let library = addOrActivateSource(emptySourceLibrary(), huya, 1);
    library = addOrActivateSource(library, bilibili, 2);
    library = addOrActivateSource(library, huya, 3);
    expect(library.favorites).toHaveLength(2);
    expect(library.activeId).toBe(huya.id);
    expect(library.favorites[0].lastPlayedAt).toBe(3);

    library = activateFavorite(library, bilibili.id, 4);
    expect(library.favorites[0].id).toBe(bilibili.id);
    expect(activeFavorite(library)?.source).toBe(bilibili.source);
  });

  it("does not change playback selection when a favorite is removed", () => {
    const huya = normalizeLiveSource("https://huya.com/196645");
    const bilibili = normalizeLiveSource("https://live.bilibili.com/5050");
    if (!huya.ok || !bilibili.ok) throw new Error("test source should normalize");
    let library = addOrActivateSource(emptySourceLibrary(), huya, 1);
    library = addOrActivateSource(library, bilibili, 2);
    library = removeFavorite(library, bilibili.id);
    expect(activeFavorite(library)).toBeUndefined();
    expect(library.favorites[0]?.id).toBe(huya.id);
  });

  it("adds a favorite without making it the playing source", () => {
    const huya = normalizeLiveSource("https://huya.com/196645");
    const bilibili = normalizeLiveSource("https://live.bilibili.com/5050");
    if (!huya.ok || !bilibili.ok) throw new Error("test source should normalize");
    let library = addOrActivateSource(emptySourceLibrary(), huya, 1);
    library = addFavoriteSource(library, bilibili, 2);
    expect(library.favorites).toHaveLength(2);
    expect(activeFavorite(library)?.id).toBe(huya.id);
    library = deactivateFavorite(library);
    expect(activeFavorite(library)).toBeUndefined();
    expect(library.favorites).toHaveLength(2);
  });

  it("persists the last played source separately from favorites", () => {
    const storage = memoryStorage();
    const library = emptySourceLibrary();
    writeLastPlayedSource(storage, "https://huya.com/196645?from=share");
    expect(storage.getItem(LAST_PLAYED_SOURCE_STORAGE_KEY)).toBe("https://www.huya.com/196645");
    expect(readLastPlayedSource(storage, library)?.source).toBe("https://www.huya.com/196645");
    expect(library.favorites).toHaveLength(0);
  });

  it("persists sanitized playback metadata", () => {
    const source = normalizeLiveSource("https://live.bilibili.com/5050");
    if (!source.ok) throw new Error("test source should normalize");
    let library = addOrActivateSource(emptySourceLibrary(), source, 1);
    library = updateActiveFavoriteMetadata(library, {
      platform: "bilibili",
      platformLabel: "Bilibili",
      roomId: "5050",
      anchor: "主播",
      title: "直播标题",
      avatarUrl: "https://i0.hdslb.com/avatar.jpg",
    });
    const storage = memoryStorage();
    writeSourceLibrary(storage, library);
    expect(storage.getItem(SOURCE_LIBRARY_STORAGE_KEY)).toContain("直播标题");
    expect(activeFavorite(readSourceLibrary(storage))?.anchor).toBe("主播");
  });
});

describe("Douyu and Douyin sources", () => {
  it.each([
    ["https://m.douyu.com/9999?dyshid=tracking", "douyu:9999", "https://www.douyu.com/9999"],
    ["https://www.douyu.com/topic/event?rid=4921614", "douyu:4921614", "https://www.douyu.com/4921614"],
    ["来看直播 https://live.douyin.com/123456789，正在直播", "douyin:123456789", "https://live.douyin.com/123456789"],
    ["分享直播：https://v.douyin.com/AbCd12/ 复制打开", "douyin:share:AbCd12", "https://v.douyin.com/AbCd12/"],
  ])("normalizes %s", (input, id, source) => {
    expect(normalizeLiveSource(input)).toMatchObject({ ok: true, id, source });
  });
  it.each([
    "https://douyu.com.evil.test/123", "https://live.douyin.com.evil.test/123",
    "https://www.douyu.com/topic/event", "https://live.douyin.com/not-a-room",
    "https://live.douyin.com:8080/123", "https://www.douyin.com/video/123",
    "https://user:password@www.douyu.com/123",
  ])("rejects unsupported source %s", (input) => {
    expect(normalizeLiveSource(input).ok).toBe(false);
  });
  it("persists new platforms alongside existing favorites", () => {
    const storage = memoryStorage();
    let library = emptySourceLibrary();
    for (const source of ["https://www.huya.com/1", "https://live.bilibili.com/2", "https://www.douyu.com/3", "https://live.douyin.com/4"]) {
      const normalized = normalizeLiveSource(source);
      if (!normalized.ok) throw new Error(normalized.message);
      library = addOrActivateSource(library, normalized, 1);
    }
    library = updateActiveFavoriteMetadata(library, { platform: "douyin", anchor: "抖音主播" });
    writeSourceLibrary(storage, library);
    expect(readSourceLibrary(storage)).toEqual(library);
    expect(readSourceLibrary(storage).favorites.map(f => f.platform)).toEqual(["douyin", "douyu", "bilibili", "huya"]);
  });
});
