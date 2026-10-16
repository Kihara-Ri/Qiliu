import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import Hls, {
  type LoaderConfiguration,
  type LoaderContext,
  type LoaderResponse,
  type PlaylistLoaderContext,
} from "hls.js";
import {
  HuyaHlsLease,
  checkedHuyaPlaylistUrl,
  huyaHlsPlaylistConfig,
} from "./huya-hls-playlist-loader";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const initial = "https://hs.hls.huya.com/src/demo.m3u8?wsSecret=old&wsTime=fffffff";
const fresh = "https://hs.hls.huya.com/src/demo.m3u8?wsSecret=new&wsTime=fffffff";
const lease = () => new HuyaHlsLease("https://www.huya.com/691406", "HS", 0, initial);
const settle = async () => { for (let i = 0; i < 15; i++) await Promise.resolve(); };

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-10-15T14:00:00Z"));
  vi.mocked(invoke).mockResolvedValue(fresh);
});
afterEach(() => { vi.unstubAllGlobals(); vi.clearAllMocks(); vi.useRealTimers(); });

it("validates signed HLS URLs and rejects non-Huya/CDN/HTTP URLs", () => {
  expect(checkedHuyaPlaylistUrl(initial)).toBe(initial);
  for (const bad of [
    "https://evil.test/demo.m3u8?wsSecret=1&wsTime=2",
    "http://hs.hls.huya.com/demo.m3u8?wsSecret=1&wsTime=2",
    "https://hs.hls.huya.com/demo.flv?wsSecret=1&wsTime=2",
    "https://hs.hls.huya.com/demo.m3u8?wsSecret=1",
  ]) expect(() => checkedHuyaPlaylistUrl(bad)).toThrow();
});

it("reuses the first lease until it is due, then coalesces concurrent renewals", async () => {
  const session = lease();
  expect(await session.currentUrl()).toBe(initial);
  expect(invoke).not.toHaveBeenCalled();
  vi.advanceTimersByTime(45_001);
  const [one, two] = await Promise.all([session.currentUrl(), session.currentUrl()]);
  expect(one).toBe(initial);
  expect(two).toBe(initial);
  expect(invoke).toHaveBeenCalledOnce();
  await settle();
  expect(invoke).toHaveBeenCalledWith("renew_huya_hls_lease", {
    source: "https://www.huya.com/691406", cdnType: "HS", bitrate: 0,
  });
  expect(session.renewals).toBe(1);
  expect(await session.currentUrl()).toBe(fresh);
  expect(invoke).toHaveBeenCalledOnce();
});

it("does not replace a working lease after failed renewal, then allows retry", async () => {
  const session = lease();
  vi.advanceTimersByTime(45_001);
  vi.mocked(invoke).mockRejectedValueOnce(new Error("timeout"));
  expect(await session.currentUrl()).toBe(initial);
  await settle();
  expect(session.renewals).toBe(0);
  expect(await session.currentUrl()).toBe(initial);
  expect(invoke).toHaveBeenCalledOnce();
  vi.advanceTimersByTime(10_001);
  expect(await session.currentUrl()).toBe(initial);
  await settle();
  expect(await session.currentUrl()).toBe(fresh);
  expect(session.renewals).toBe(1);
});

it("does not hold back playlist requests during slow proactive renewal", async () => {
  const session = lease();
  let finish!: (url: string) => void;
  vi.mocked(invoke).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  vi.advanceTimersByTime(45_001);
  expect(await session.currentUrl()).toBe(initial);
  expect(await session.currentUrl()).toBe(initial);
  expect(invoke).toHaveBeenCalledOnce();
  finish(fresh);
  await settle();
  expect(await session.currentUrl()).toBe(fresh);
  expect(session.renewals).toBe(1);
});

it("rejects late renewals after source changes or playback stops", async () => {
  const session = lease();
  let finish!: (url: string) => void;
  vi.mocked(invoke).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  const pending = session.currentUrl(true);
  session.close();
  finish(fresh);
  await expect(pending).rejects.toThrow("已结束");
  await expect(session.currentUrl()).rejects.toThrow("已结束");
});

function makeRequestConfig(): LoaderConfiguration {
  return {
    loadPolicy: {
      maxTimeToFirstByteMs: 5_000, maxLoadTimeMs: 8_000,
      errorRetry: null, timeoutRetry: null,
    },
    timeout: 8_000, maxRetry: 0, retryDelay: 0, maxRetryDelay: 0,
  };
}

it("retries an expired playlist 403 with a fresh URL without rebuilding Hls.js", async () => {
  vi.useRealTimers();
  vi.stubGlobal("self", globalThis);
  const attempted: string[] = [];
  vi.stubGlobal("fetch", vi.fn(async (request: Request) => {
    attempted.push(request.url);
    const old = request.url.includes("wsSecret=old");
    const response = new Response(old ? "expired" : "#EXTM3U\n#EXT-X-TARGETDURATION:4\n", {
      status: old ? 403 : 200,
      headers: { "content-type": "application/vnd.apple.mpegurl" },
    });
    Object.defineProperty(response, "url", { value: request.url });
    return response;
  }));

  const session = lease();
  const Factory = huyaHlsPlaylistConfig(session).pLoader!;
  const loader = new Factory(Hls.DefaultConfig);
  const context = {
    url: initial,
    responseType: "text",
    type: "manifest" as LoaderContext["type"],
  } as PlaylistLoaderContext;

  const response = new Promise<LoaderResponse>((resolve, reject) => {
    loader.load(context, makeRequestConfig(), {
      onSuccess: (data) => resolve(data),
      onError: error => reject(new Error(error.text)),
      onTimeout: () => reject(new Error("loader timeout")),
    });
  });
  const result = await response;
  expect(attempted).toHaveLength(2);
  expect(attempted[0]).toBe(initial);
  expect(attempted[1]).toBe(fresh);
  expect(result.url).toBe(fresh);
  expect(result.data).toContain("#EXTM3U");
  expect(session.renewals).toBe(1);
  expect(invoke).toHaveBeenCalledOnce();
  // hls.js retains the original playlist context; only network lease changed.
  expect(context.url).toBe(initial);
  loader.destroy();
});

it("does not start a late playlist load after it has been destroyed", async () => {
  vi.useRealTimers();
  vi.stubGlobal("self", globalThis);
  let finish!: (url: string) => void;
  vi.mocked(invoke).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  const session = lease();
  const Factory = huyaHlsPlaylistConfig(session).pLoader!;
  const loader = new Factory(Hls.DefaultConfig);
  const fetch = vi.fn();
  vi.stubGlobal("fetch", fetch);
  // Expire the cached signature, causing load to await Rust renewal.
  session.close();
  const error = vi.fn();
  loader.load({
    url: initial,
    responseType: "text",
    type: "manifest" as LoaderContext["type"],
  } as PlaylistLoaderContext, makeRequestConfig(), {
    onSuccess: vi.fn(), onError: error, onTimeout: vi.fn(),
  });
  loader.destroy();
  await settle();
  expect(fetch).not.toHaveBeenCalled();
  expect(error).not.toHaveBeenCalled();
  expect(finish).toBeUndefined();
});
