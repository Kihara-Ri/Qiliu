import { invoke } from "@tauri-apps/api/core";
import { FetchLoader } from "hls.js";
import type {
  HlsConfig,
  Loader,
  LoaderCallbacks,
  LoaderConfiguration,
  LoaderStats,
  PlaylistLoaderContext,
} from "hls.js";

const RENEW_AFTER_MS = 45_000;

/**
 * A signed Huya URL is a short-lived network lease, not the identity of the
 * playback session. Renewing it must never touch video, Hls.js or MediaSource.
 */
export class HuyaHlsLease {
  private url: string;
  private renewedAt: number;
  private retryAfter = 0;
  private pending: Promise<string> | undefined;
  private closed = false;
  private renewalCount = 0;

  constructor(
    private readonly source: string,
    private readonly cdnType: string,
    private readonly bitrate: number,
    initialUrl: string,
  ) {
    this.url = checkedHuyaPlaylistUrl(initialUrl);
    this.renewedAt = Date.now();
  }

  get renewals(): number { return this.renewalCount; }

  async currentUrl(force = false): Promise<string> {
    if (this.closed) throw new Error("虎牙 HLS 会话已结束");
    if (force) return this.renew();

    // Critical: a normal playlist refresh must never await metadata calls,
    // signature generation or segment preflight. Keep playing on the old lease
    // while renewing asynchronously; publication is atomic after validation.
    if (Date.now() - this.renewedAt >= RENEW_AFTER_MS
        && Date.now() >= this.retryAfter && !this.pending) {
      void this.renew().catch(() => {
        // Avoid hammering profileRoom when a CDN/API outage persists.
        this.retryAfter = Date.now() + 10_000;
      });
    }
    return this.url;
  }

  private async renew(): Promise<string> {
    if (this.closed) throw new Error("虎牙 HLS 会话已结束");
    if (this.pending) return this.pending;
    const request = invoke<string>("renew_huya_hls_lease", {
      source: this.source,
      cdnType: this.cdnType,
      bitrate: this.bitrate,
    }).then(signedUrl => {
      const url = checkedHuyaPlaylistUrl(signedUrl);
      if (this.closed) throw new Error("虎牙 HLS 会话已结束");
      this.url = url;
      this.renewedAt = Date.now();
      this.retryAfter = 0;
      this.renewalCount++;
      return url;
    });
    this.pending = request;
    try { return await request; }
    finally {
      if (this.pending === request) this.pending = undefined;
    }
  }

  close(): void { this.closed = true; }
}

export function checkedHuyaPlaylistUrl(value: string): string {
  const url = new URL(value);
  if (url.protocol !== "https:" ||
      (url.hostname !== "huya.com" && !url.hostname.endsWith(".huya.com")) ||
      !url.pathname.endsWith(".m3u8") ||
      !url.searchParams.has("wsSecret") ||
      !url.searchParams.has("wsTime")) {
    throw new Error("虎牙返回的 HLS 地址无效");
  }
  return url.href;
}

/**
 * hls.js invokes pLoader on every media-playlist refresh. The response URL
 * remains the actual CDN URL so hls.js resolves relative TS segments correctly.
 * Neither normal renewal nor HTTP 403 retry causes a new Hls instance.
 */
export function huyaHlsPlaylistConfig(lease: HuyaHlsLease): Pick<HlsConfig, "pLoader"> {
  return {
    pLoader: class HuyaPlaylistLoader implements Loader<PlaylistLoaderContext> {
      context: PlaylistLoaderContext | null = null;
      stats: LoaderStats;
      private loader: FetchLoader | undefined;
      private cancelled = false;

      constructor(private readonly config: HlsConfig) {
        this.loader = new FetchLoader(config);
        this.stats = this.loader.stats;
      }

      load(
        context: PlaylistLoaderContext,
        config: LoaderConfiguration,
        callbacks: LoaderCallbacks<PlaylistLoaderContext>,
      ): void {
        this.context = context;
        void this.loadAttempt(context, config, callbacks, false);
      }

      private async loadAttempt(
        context: PlaylistLoaderContext,
        config: LoaderConfiguration,
        callbacks: LoaderCallbacks<PlaylistLoaderContext>,
        force: boolean,
      ): Promise<void> {
        let signedUrl: string;
        try {
          signedUrl = await lease.currentUrl(force);
        } catch (error) {
          if (!this.cancelled) {
            callbacks.onError({ code: 0, text: String(error) }, context, null, this.stats);
          }
          return;
        }
        if (this.cancelled) return;

        // This context is private to the network loader: hls.js retains its
        // original playlist context and receives the actual response.url.
        const requestContext = { ...context, url: signedUrl };
        const loader = new FetchLoader({
          ...this.config,
          fetchSetup: (ctx, init) => new Request(ctx.url, { ...init, cache: "no-store" }),
        });
        this.loader = loader;
        this.stats = loader.stats;
        loader.load(requestContext, config, {
          ...callbacks,
          onSuccess: (response, stats, _requestContext, network) => {
            if (!this.cancelled) callbacks.onSuccess(response, stats, context, network);
          },
          onError: (response, _requestContext, network, stats) => {
            if (this.cancelled) return;
            // An expired signature is often answered by a CDN error page without CORS
            // headers, which fetch reports as status 0 rather than 403.
            if (!force && (response.code === 0 || response.code === 401 || response.code === 403)) {
              loader.destroy();
              if (this.loader === loader) this.loader = undefined;
              void this.loadAttempt(context, config, callbacks, true);
            } else {
              callbacks.onError(response, context, network, stats);
            }
          },
          onTimeout: (stats, _requestContext, network) => {
            if (!this.cancelled) callbacks.onTimeout(stats, context, network);
          },
          onAbort: (stats, _requestContext, network) => {
            if (!this.cancelled) callbacks.onAbort?.(stats, context, network);
          },
          onProgress: (stats, _requestContext, data, network) => {
            if (!this.cancelled) callbacks.onProgress?.(stats, context, data, network);
          },
        });
      }

      abort(): void {
        this.cancelled = true;
        this.loader?.abort();
      }

      destroy(): void {
        this.cancelled = true;
        this.loader?.destroy();
        this.loader = undefined;
        this.context = null;
      }

      getCacheAge(): number | null { return this.loader?.getCacheAge() ?? null; }
      getResponseHeader(name: string): string | null {
        return this.loader?.getResponseHeader(name) ?? null;
      }
    },
  };
}

/**
 * Huya's sliding window and signed URLs produce short bursts of 404/403/timeouts.
 * Retry them inside the same Hls.js session instead of surfacing a fatal error.
 */
export function huyaHlsRetryConfig(): Pick<
  HlsConfig,
  "playlistLoadPolicy" | "fragLoadPolicy" | "manifestLoadPolicy"
> {
  const policy = (timeoutRetries: number, errorRetries: number) => ({
    default: {
      maxTimeToFirstByteMs: 10_000,
      maxLoadTimeMs: 20_000,
      timeoutRetry: { maxNumRetry: timeoutRetries, retryDelayMs: 0, maxRetryDelayMs: 0 },
      errorRetry: {
        maxNumRetry: errorRetries,
        retryDelayMs: 500,
        maxRetryDelayMs: 4_000,
        backoff: "exponential" as const,
      },
    },
  });
  return {
    manifestLoadPolicy: policy(4, 6),
    playlistLoadPolicy: policy(6, 10),
    fragLoadPolicy: policy(6, 8),
  };
}
