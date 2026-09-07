import { expect, it } from "vitest";
import { FetchLoader, type LoaderContext } from "hls.js";
import { bilibiliLivePlaylistConfig } from "./hls-playlist-policy";

it("bypasses cached live playlists without changing the signed URL or cancellation", async () => {
  const config = bilibiliLivePlaylistConfig();
  const controller = new AbortController();
  const url = "https://test.bilivideo.com/live/index.m3u8?sign=a%2Bb&expires=123";
  // hls.js publishes this discriminator as an ambient const enum.
  const request = await config.fetchSetup!({ url, responseType: "text", type: "manifest" as LoaderContext["type"] }, {
    signal: controller.signal, credentials: "same-origin", mode: "cors",
  });
  expect(config.pLoader!.prototype).toBeInstanceOf(FetchLoader);
  expect(request.url).toBe(url);
  expect(request.cache).toBe("no-store");
  expect(request.credentials).toBe("same-origin");
  expect(request.mode).toBe("cors");
  expect([...request.headers]).toEqual([]);
  controller.abort();
  expect(request.signal.aborted).toBe(true);
  expect(config).not.toHaveProperty("loader");
  expect(config).not.toHaveProperty("fLoader");
});
