import { FetchLoader, type HlsConfig, type PlaylistLoaderContext } from "hls.js";

class FetchPlaylistLoader extends FetchLoader {
  // hls.js supplies playlist contexts to pLoader; FetchLoader is otherwise generic.
  declare context: PlaylistLoaderContext | null;
}

/** Live Bilibili playlists may advertise max-age=300 despite changing each second. */
export function bilibiliLivePlaylistConfig(): Pick<HlsConfig, "pLoader" | "fetchSetup"> {
  return {
    // Override only playlist loading; media segments keep the normal loader/cache.
    pLoader: FetchPlaylistLoader,
    fetchSetup: (context, init) => new Request(context.url, {
      ...init,
      cache: "no-store",
    }),
  };
}
