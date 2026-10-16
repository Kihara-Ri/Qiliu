import { invoke } from "@tauri-apps/api/core";
import type {
  HlsConfig,
  Loader,
  LoaderCallbacks,
  LoaderConfiguration,
  LoaderStats,
  PlaylistLoaderContext,
} from "hls.js";

function emptyStats(): LoaderStats {
  return {
    aborted: false,
    loaded: 0,
    retry: 0,
    total: 0,
    chunkCount: 0,
    bwEstimate: 0,
    loading: { start: 0, first: 0, end: 0 },
    parsing: { start: 0, end: 0 },
    buffering: { start: 0, first: 0, end: 0 },
  };
}

/**
 * Twitch's playlist CDN rejects the `http://tauri.localhost` Origin used by
 * Windows and Android WebViews (HTTP 403). Playlists are therefore fetched by
 * Rust, which has no Origin; the much larger media segments still go straight
 * from the CDN to Hls.js.
 */
export function twitchHlsPlaylistConfig(): Pick<HlsConfig, "pLoader"> {
  return {
    pLoader: class TwitchPlaylistLoader implements Loader<PlaylistLoaderContext> {
      context: PlaylistLoaderContext | null = null;
      stats: LoaderStats = emptyStats();
      private done = false;

      load(
        context: PlaylistLoaderContext,
        _config: LoaderConfiguration,
        callbacks: LoaderCallbacks<PlaylistLoaderContext>,
      ): void {
        this.context = context;
        this.stats = emptyStats();
        this.done = false;
        const stats = this.stats;
        stats.loading.start = performance.now();
        invoke<string>("fetch_twitch_playlist", { url: context.url }).then(
          (text) => {
            if (this.done) return;
            this.done = true;
            stats.loading.first = stats.loading.end = performance.now();
            stats.loaded = stats.total = text.length;
            callbacks.onSuccess({ url: context.url, data: text }, stats, context, null);
          },
          (error: unknown) => {
            if (this.done) return;
            this.done = true;
            stats.loading.end = performance.now();
            const text = String(error);
            const code = Number(/HTTP (\d{3})/.exec(text)?.[1] ?? 0);
            callbacks.onError({ code, text }, context, null, stats);
          },
        );
      }

      abort(): void {
        this.done = true;
        this.stats.aborted = true;
      }

      destroy(): void {
        this.done = true;
        this.context = null;
      }

      getCacheAge(): number | null { return null; }
      getResponseHeader(): string | null { return null; }
    },
  };
}
