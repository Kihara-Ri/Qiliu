import { invoke } from "@tauri-apps/api/core";
import Hls, {
  ErrorDetails,
  ErrorTypes,
  type ErrorData,
  type FragLoadedData,
} from "hls.js";
import mpegts from "mpegts.js";
import {
  BILIBILI_BUFFER_WINDOW_MS,
  decideRecoveryLine,
  recordBilibiliBufferEvent,
  shouldPreferStableBilibiliQuality,
} from "./playback-recovery-policy";

export type PlaybackPhase =
  | "idle"
  | "resolving"
  | "connecting"
  | "playing"
  | "replay"
  | "paused"
  | "recovering"
  | "offline"
  | "error";

export interface PlaybackViewState {
  phase: PlaybackPhase;
  headline: string;
  detail: string;
  mutedByPolicy: boolean;
}

export interface PlaybackStats {
  active: boolean;
  live: boolean;
  replay: boolean;
  platform: string;
  platformLabel: string;
  sourceUrl: string;
  roomId: string;
  anchor: string;
  title: string;
  avatarUrl: string;
  qualityLabel: string;
  nominalBitrateKbps: number;
  measuredBitrateKbps: number | null;
  resolution: string;
  frameRate: number | null;
  bufferedSeconds: number | null;
  droppedFrames: number | null;
  totalFrames: number | null;
  lineLabel: string;
  connectionHealth: string;
}

interface LiveStream {
  platform: string;
  platformLabel: string;
  sourceUrl: string;
  roomId: string;
  title: string;
  anchor: string;
  avatarUrl: string;
  isLive: boolean;
  isReplay: boolean;
  url: string | null;
  lineIndex: number;
  lineCount: number;
  lineName: string;
  qualityLabel: string;
  bitrate: number;
  startPositionSeconds: number;
  format: string;
}

type StateListener = (state: PlaybackViewState) => void;
type StatsListener = (stats: PlaybackStats) => void;

const SOURCE_QUALITY_BITRATE_KBPS = 0;
const FALLBACK_BITRATE_KBPS = 4_000;
const SOFT_STALL_LIMIT_MS = 12_000;
const STALL_LIMIT_MS = 30_000;
const SOFT_RECOVERY_GRACE_MS = 15_000;
const CONNECT_TIMEOUT_MS = 20_000;
const OFFLINE_RECHECK_MS = 30_000;
const STABLE_RESET_MS = 60_000;
const HUYA_CONTINUATION_PREFETCH_MS = 100_000;
const RECONNECT_DELAYS_MS = [0, 1_000, 2_000, 4_000, 8_000, 15_000] as const;

export class PlaybackSupervisor {
  private source = "";
  private generation = 0;
  private attempt = 0;
  private lineCursor = 0;
  private connecting = false;
  private userPaused = false;
  private mutedByPolicy = false;
  private suppressMediaEventsUntil = 0;
  private lastMediaTime = 0;
  private lastAdvanceAt = Date.now();
  private lastFrameAdvanceAt = Date.now();
  private softRecoveryAt = 0;
  private reconnectTimer: number | undefined;
  private stableTimer: number | undefined;
  private continuationPrefetchTimer: number | undefined;
  private readonly watchdogTimer: number;
  private readonly statsTimer: number;
  private currentStream: LiveStream | undefined;
  private hls: Hls | undefined;
  private mpegtsPlayer: ReturnType<typeof mpegts.createPlayer> | undefined;
  private hlsMediaRecoveryCount = 0;
  private lastFrameCount: number | undefined;
  private lastStatsAt = performance.now();
  private measuredBitrateKbps: number | null = null;
  private bitrateSamples: number[] = [];
  private failuresOnCurrentLine = 0;
  private lastFailureAt = 0;
  private reconnectCount = 0;
  private lineSwitchCount = 0;
  private lastRecoveryReason = "";
  private lastRecoveryAction = "";
  private continuationCount = 0;
  private preparedContinuation: LiveStream | undefined;
  private silentContinuation = false;
  private preferStableBilibiliQuality = false;
  private recentBilibiliBufferEvents: number[] = [];
  private lastBilibiliBufferEventAt = 0;
  private bilibiliBufferEventCount = 0;
  private bilibiliQualityFallbackCount = 0;

  constructor(
    private readonly video: HTMLVideoElement,
    private readonly onState: StateListener,
    private readonly onStats: StatsListener,
  ) {
    this.video.addEventListener("playing", this.handlePlaying);
    this.video.addEventListener("timeupdate", this.handleProgress);
    this.video.addEventListener("waiting", this.handleWaiting);
    this.video.addEventListener("stalled", this.handleWaiting);
    this.video.addEventListener("error", this.handleMediaFailure);
    this.video.addEventListener("ended", this.handleMediaFailure);
    window.addEventListener("online", this.handleOnline);
    window.addEventListener("offline", this.handleOffline);
    document.addEventListener("visibilitychange", this.handleVisibilityChange);
    this.watchdogTimer = window.setInterval(this.watchPlayback, 3_000);
    this.statsTimer = window.setInterval(this.collectStats, 1_000);
    this.emitEmptyStats();
  }

  setStatsInspectionActive(active: boolean): void {
    if (active) this.collectStats();
  }

  start(source: string): void {
    this.generation += 1;
    this.source = source;
    this.attempt = 0;
    this.lineCursor = 0;
    this.connecting = false;
    this.userPaused = false;
    this.mutedByPolicy = false;
    this.currentStream = undefined;
    this.resetRecoveryHistory();
    this.resetStatsSample();
    this.clearTimers();
    this.resetMedia();
    void this.connect(this.generation);
  }

  stop(): void {
    this.generation += 1;
    this.source = "";
    this.connecting = false;
    this.currentStream = undefined;
    this.resetRecoveryHistory();
    this.resetStatsSample();
    this.clearTimers();
    this.resetMedia();
    this.emit("idle", "等待直播源", "使用右上角的设置按钮添加直播源收藏");
  }

  async togglePause(): Promise<void> {
    if (!this.video.src) return;
    if (this.video.paused) {
      this.userPaused = false;
      try {
        await this.video.play();
        this.handlePlaying();
      } catch (error) {
        this.scheduleRecovery(errorMessage(error));
      }
      return;
    }

    this.userPaused = true;
    this.video.pause();
    this.emit("paused", "已暂停", "按空格继续播放");
  }

  async unlockSound(): Promise<void> {
    await this.setMuted(false);
  }

  async setMuted(muted: boolean): Promise<void> {
    const wasMutedByPolicy = this.mutedByPolicy;
    this.video.muted = muted;
    this.mutedByPolicy = false;

    if (muted) return;

    try {
      if (this.video.src && !this.userPaused) {
        await this.video.play();
      }
      if (wasMutedByPolicy && this.currentStream) {
        this.emitPlayingState();
      }
    } catch {
      this.video.muted = true;
      this.mutedByPolicy = true;
      this.emitPlayingState();
    }
  }

  destroy(): void {
    this.stop();
    window.clearInterval(this.watchdogTimer);
    window.clearInterval(this.statsTimer);
    this.video.removeEventListener("playing", this.handlePlaying);
    this.video.removeEventListener("timeupdate", this.handleProgress);
    this.video.removeEventListener("waiting", this.handleWaiting);
    this.video.removeEventListener("stalled", this.handleWaiting);
    this.video.removeEventListener("error", this.handleMediaFailure);
    this.video.removeEventListener("ended", this.handleMediaFailure);
    window.removeEventListener("online", this.handleOnline);
    window.removeEventListener("offline", this.handleOffline);
    document.removeEventListener("visibilitychange", this.handleVisibilityChange);
  }

  private async connect(
    generation: number,
    silentContinuation = false,
    preparedStream?: LiveStream,
  ): Promise<void> {
    if (generation !== this.generation || !this.source || this.connecting) return;
    if (!navigator.onLine) {
      this.handleOffline();
      return;
    }

    this.connecting = true;
    this.silentContinuation = silentContinuation;
    const bitrate = this.preferStableBilibiliQuality || this.attempt >= 3
      ? FALLBACK_BITRATE_KBPS
      : SOURCE_QUALITY_BITRATE_KBPS;
    const platformLabel = platformLabelForSource(this.source);
    if (!silentContinuation) {
      this.emit("resolving", "正在取得直播信号", `${platformLabel} · H.264 · 最高可用画质`);
    }

    try {
      const stream = preparedStream ?? await invoke<LiveStream>("resolve_live_stream", {
          source: this.source,
          lineIndex: this.lineCursor,
          bitrate,
        });
      if (generation !== this.generation) return;
      this.currentStream = stream;
      this.collectStats();

      if (!stream.url) {
        this.connecting = false;
        this.silentContinuation = false;
        this.emit(
          "offline",
          stream.isReplay ? stream.title || "回放暂不可用" : stream.title || "直播暂未开始",
          stream.isReplay
            ? "暂未取得回放地址，应用会自动重试"
            : "应用会保持等待，开播后自动连接",
        );
        this.reconnectTimer = window.setTimeout(
          () => {
            this.reconnectTimer = undefined;
            void this.connect(generation);
          },
          OFFLINE_RECHECK_MS,
        );
        return;
      }

      if (!silentContinuation) {
        this.emit(
          "connecting",
          stream.isReplay ? "正在载入回放" : "正在建立播放",
          stream.title ||
            (stream.isReplay
              ? `${stream.anchor || stream.platformLabel} · 录像回放`
              : `${stream.anchor || stream.platformLabel} · 线路 ${stream.lineIndex + 1}`),
        );
      }
      await this.playUrl(stream.url, stream.isReplay ? stream.startPositionSeconds : 0);
      if (generation !== this.generation) return;
      this.connecting = false;
      this.handlePlaying();
      this.silentContinuation = false;
    } catch (error) {
      if (generation !== this.generation) return;
      this.connecting = false;
      this.silentContinuation = false;
      if (isUnsupportedMedia(error)) {
        this.emit("error", "当前系统不支持直播解码", "此系统不支持所需的 MSE / H.264 播放能力");
        return;
      }
      this.scheduleRecovery(errorMessage(error));
    }
  }

  private async playUrl(url: string, startPositionSeconds: number): Promise<void> {
    this.suppressMediaEventsUntil = Date.now() + 500;
    this.destroyPlayers();
    this.video.pause();
    this.video.loop = false;
    this.video.removeAttribute("src");
    this.video.load();
    this.lastMediaTime = 0;
    this.lastAdvanceAt = Date.now();
    this.lastFrameAdvanceAt = Date.now();
    this.softRecoveryAt = 0;
    this.resetStatsSample();

    try {
      if (this.currentStream?.format === "flv") {
        await withTimeout(this.attachMpegts(url), CONNECT_TIMEOUT_MS);
      } else if (Hls.isSupported()) {
        await withTimeout(this.attachHls(url), CONNECT_TIMEOUT_MS);
      } else {
        this.video.src = url;
        this.video.load();
      }
      if (startPositionSeconds > 0) {
        await withTimeout(waitForMediaMetadata(this.video), CONNECT_TIMEOUT_MS);
        const maximum = Number.isFinite(this.video.duration)
          ? Math.max(0, this.video.duration - 0.5)
          : startPositionSeconds;
        this.video.currentTime = Math.min(startPositionSeconds, maximum);
      }
      await withTimeout(this.video.play(), CONNECT_TIMEOUT_MS);
    } catch (error) {
      if (!isAutoplayBlocked(error)) throw error;
      this.video.muted = true;
      this.mutedByPolicy = true;
      await withTimeout(this.video.play(), CONNECT_TIMEOUT_MS);
    }
  }

  private attachHls(url: string): Promise<void> {
    return new Promise<void>((resolve, reject) => {
      const hls = new Hls({
        enableWorker: true,
        lowLatencyMode: false,
        maxBufferLength: 12,
        maxMaxBufferLength: 24,
        backBufferLength: 2,
        liveSyncDurationCount: 3,
        liveMaxLatencyDurationCount: 8,
        maxLiveSyncPlaybackRate: 1.08,
      });
      this.hls = hls;
      this.hlsMediaRecoveryCount = 0;
      let settled = false;

      const rejectInitialLoad = (data: ErrorData): void => {
        if (settled || !data.fatal || this.hls !== hls) return;
        settled = true;
        reject(new Error(hlsErrorMessage(data)));
      };

      hls.on(Hls.Events.MEDIA_ATTACHED, () => {
        if (this.hls === hls) hls.loadSource(url);
      });
      hls.on(Hls.Events.MANIFEST_PARSED, () => {
        if (settled || this.hls !== hls) return;
        settled = true;
        resolve();
      });
      hls.on(Hls.Events.FRAG_LOADED, (_event, data) => this.handleHlsFragment(hls, data));
      hls.on(Hls.Events.ERROR, (_event, data) => {
        rejectInitialLoad(data);
        this.handleHlsError(hls, data);
      });
      hls.attachMedia(this.video);
    });
  }

  private attachMpegts(url: string): Promise<void> {
    if (!mpegts.isSupported()) {
      return Promise.reject(new DOMException("MSE FLV is not supported", "NotSupportedError"));
    }

    return new Promise<void>((resolve, reject) => {
      const isBilibiliLive = this.currentStream?.platform === "bilibili"
        && this.currentStream.isLive;
      const player = mpegts.createPlayer(
        {
          type: "flv",
          isLive: true,
          cors: true,
          withCredentials: false,
          hasAudio: true,
          hasVideo: true,
          url,
        },
        {
          // Bilibili original quality can deliver many small FLV chunks. Move
          // transmuxing off the WebView UI thread so MSE appends do not fall
          // behind while the network itself is still healthy.
          enableWorker: isBilibiliLive,
          enableStashBuffer: true,
          // Keep a small jitter cushion without delaying the first frame with
          // a large startup stash.
          // Give Bilibili roughly a few seconds of startup cushion. Huya keeps
          // the smaller value for fast same-line continuation at natural EOF.
          stashInitialSize: (isBilibiliLive ? 1_024 : 256) * 1_024,
          lazyLoad: false,
          autoCleanupSourceBuffer: true,
          // Use mpegts.js' conservative live defaults. Keeping only 15 seconds
          // behind the playhead caused WKWebView to remove SourceBuffer ranges
          // too close to active playback roughly every 45 seconds.
          autoCleanupMaxBackwardDuration: 3 * 60,
          autoCleanupMinBackwardDuration: 2 * 60,
          liveSync: false,
          statisticsInfoReportInterval: 1_000,
          fixAudioTimestampGap: true,
        },
      );
      this.mpegtsPlayer = player;
      let settled = false;

      player.on(mpegts.Events.MEDIA_INFO, () => {
        if (settled || this.mpegtsPlayer !== player) return;
        settled = true;
        resolve();
      });
      player.on(mpegts.Events.STATISTICS_INFO, (statistics: unknown) => {
        if (this.mpegtsPlayer !== player || !isRecord(statistics)) return;
        const speed = statistics.speed;
        if (typeof speed === "number" && Number.isFinite(speed) && speed > 0) {
          this.measuredBitrateKbps = speed * 8;
        }
      });
      player.on(mpegts.Events.LOADING_COMPLETE, () => {
        if (this.mpegtsPlayer !== player) return;
        this.handleMpegtsLoadingComplete();
      });
      player.on(
        mpegts.Events.ERROR,
        (errorType: unknown, errorDetail: unknown, errorInfo: unknown) => {
          if (this.mpegtsPlayer !== player) return;
          const message = mpegtsErrorMessage(errorType, errorDetail, errorInfo);
          if (!settled) {
            settled = true;
            reject(new Error(message));
            return;
          }
          if (!this.connecting) this.scheduleRecovery(message);
        },
      );
      player.attachMediaElement(this.video);
      player.load();
    });
  }

  private handleHlsFragment(hls: Hls, data: FragLoadedData): void {
    if (this.hls !== hls || data.frag.sn === "initSegment") return;
    const loadedBytes = data.frag.stats.loaded;
    const durationSeconds = data.frag.duration;
    if (loadedBytes <= 0 || !Number.isFinite(durationSeconds) || durationSeconds <= 0) return;

    const sample = loadedBytes * 8 / durationSeconds / 1_000;
    if (!Number.isFinite(sample) || sample <= 0) return;
    this.bitrateSamples.push(sample);
    if (this.bitrateSamples.length > 3) this.bitrateSamples.shift();
    this.measuredBitrateKbps =
      this.bitrateSamples.reduce((sum, value) => sum + value, 0) / this.bitrateSamples.length;
  }

  private handleHlsError(hls: Hls, data: ErrorData): void {
    if (this.hls !== hls) return;
    if (data.details === ErrorDetails.BUFFER_STALLED_ERROR) this.handleWaiting();
    if (!data.fatal || this.connecting) return;

    if (data.type === ErrorTypes.MEDIA_ERROR && this.hlsMediaRecoveryCount === 0) {
      this.hlsMediaRecoveryCount = 1;
      this.suppressMediaEventsUntil = Date.now() + 1_000;
      this.emit("recovering", "正在修复播放", "播放器正在重新连接解码器");
      hls.recoverMediaError();
      void this.video.play().catch(() => undefined);
      return;
    }

    this.scheduleRecovery(hlsErrorMessage(data));
  }

  private scheduleHuyaContinuationPrefetch(): void {
    const stream = this.currentStream;
    if (
      this.continuationPrefetchTimer !== undefined ||
      this.preparedContinuation ||
      stream?.platform !== "huya" ||
      !stream.isLive ||
      stream.format !== "flv"
    ) {
      return;
    }

    const generation = this.generation;
    const source = this.source;
    const lineCursor = this.lineCursor;
    this.continuationPrefetchTimer = window.setTimeout(() => {
      this.continuationPrefetchTimer = undefined;
      const bitrate = this.attempt >= 3 ? FALLBACK_BITRATE_KBPS : SOURCE_QUALITY_BITRATE_KBPS;
      void invoke<LiveStream>("resolve_live_stream", {
        source,
        lineIndex: lineCursor,
        bitrate,
      }).then((prepared) => {
        if (
          generation === this.generation &&
          source === this.source &&
          lineCursor === this.lineCursor &&
          prepared.platform === "huya" &&
          prepared.isLive &&
          prepared.format === "flv" &&
          prepared.url
        ) {
          this.preparedContinuation = prepared;
        }
      }).catch(() => {
        // Natural EOF will resolve synchronously if prefetch was unavailable.
      });
    }, HUYA_CONTINUATION_PREFETCH_MS);
  }

  private handleMpegtsLoadingComplete(): void {
    const stream = this.currentStream;
    if (
      stream?.platform !== "huya" ||
      !stream.isLive ||
      stream.format !== "flv" ||
      this.connecting ||
      this.userPaused
    ) {
      return;
    }

    const prepared = this.preparedContinuation;
    this.preparedContinuation = undefined;
    window.clearTimeout(this.continuationPrefetchTimer);
    this.continuationPrefetchTimer = undefined;
    this.continuationCount += 1;
    this.lastRecoveryAction = "同线路续接";
    this.lastRecoveryReason = "虎牙长连接自然结束";
    void this.connect(this.generation, true, prepared);
  }

  private readonly handlePlaying = (): void => {
    if (!this.source || this.userPaused) return;
    this.connecting = false;
    this.lastMediaTime = this.video.currentTime;
    this.lastAdvanceAt = Date.now();
    this.lastFrameAdvanceAt = Date.now();
    this.softRecoveryAt = 0;
    window.clearTimeout(this.stableTimer);
    this.stableTimer = window.setTimeout(() => {
      this.attempt = 0;
    }, STABLE_RESET_MS);
    this.scheduleHuyaContinuationPrefetch();
    if (!this.silentContinuation) this.emitPlayingState();
  };

  private emitPlayingState(): void {
    const stream = this.currentStream;
    const headline = stream?.title || (stream?.isReplay ? "正在回放" : "正在直播");
    const pieces = (stream?.isReplay
      ? [stream.anchor, "录像回放"]
      : [
          stream?.anchor,
          stream?.qualityLabel,
          stream?.lineName ? `${stream.lineName} 线路` : "",
        ])
      .filter(Boolean)
      .join(" · ");
    const detail = this.mutedByPolicy ? "画面已播放，点一下恢复声音" : pieces;
    this.emit(stream?.isReplay ? "replay" : "playing", headline, detail, this.mutedByPolicy);
  }

  private readonly handleProgress = (): void => {
    const current = this.video.currentTime;
    if (current < this.lastMediaTime || current - this.lastMediaTime >= 0.05) {
      this.lastMediaTime = current;
      this.lastAdvanceAt = Date.now();
      this.softRecoveryAt = 0;
    }
  };

  private readonly handleWaiting = (): void => {
    if (this.connecting || this.userPaused || Date.now() < this.suppressMediaEventsUntil) return;
    const stream = this.currentStream;
    this.recordBilibiliUnderflow();
    this.emit(
      "connecting",
      "正在缓冲",
      stream?.lineCount ? `当前线路 ${stream.lineIndex + 1}/${stream.lineCount}` : "正在等待新画面",
    );
  };

  private recordBilibiliUnderflow(): void {
    const stream = this.currentStream;
    const buffered = bufferedAhead(this.video);
    if (
      stream?.platform !== "bilibili"
      || !stream.isLive
      || this.preferStableBilibiliQuality
      || buffered === null
      || buffered > 0.5
    ) {
      return;
    }
    const recorded = recordBilibiliBufferEvent(
      this.recentBilibiliBufferEvents,
      this.lastBilibiliBufferEventAt,
      Date.now(),
    );
    this.recentBilibiliBufferEvents = recorded.recentEvents;
    this.lastBilibiliBufferEventAt = recorded.lastEventAt;
    if (recorded.accepted) this.bilibiliBufferEventCount += 1;
  }

  private readonly handleMediaFailure = (): void => {
    if (
      !this.source ||
      this.connecting ||
      this.userPaused ||
      Date.now() < this.suppressMediaEventsUntil
    ) {
      return;
    }
    this.scheduleRecovery(mediaErrorMessage(this.video.error));
  };

  private readonly watchPlayback = (): void => {
    if (!this.source || !this.currentStream?.url || this.connecting || this.userPaused) return;
    this.handleProgress();
    const now = Date.now();
    const mediaStalledFor = now - this.lastAdvanceAt;
    const frameStalledFor = this.lastFrameCount === undefined
      ? mediaStalledFor
      : now - this.lastFrameAdvanceAt;
    // WKWebView's decoded-frame counter can update late or pause while media
    // time is still advancing. A hard reconnect requires both clocks to stop;
    // otherwise a stale diagnostics counter can create a false periodic stall.
    const stalledFor = Math.min(mediaStalledFor, frameStalledFor);

    if (stalledFor >= SOFT_STALL_LIMIT_MS && this.softRecoveryAt === 0) {
      this.softRecoveryAt = now;
      if (this.tryCatchUpToLiveEdge()) return;
    }

    const softRecoveryExpired =
      this.softRecoveryAt > 0 && now - this.softRecoveryAt >= SOFT_RECOVERY_GRACE_MS;
    if (stalledFor >= STALL_LIMIT_MS || softRecoveryExpired) {
      this.scheduleRecovery("画面长时间没有前进");
    }
  };

  private tryCatchUpToLiveEdge(): boolean {
    if (!this.currentStream?.isLive || this.currentStream?.format !== "flv") return false;

    try {
      const ranges = this.video.seekable;
      if (ranges.length === 0) return false;
      const liveEdge = ranges.end(ranges.length - 1);
      const target = Math.max(0, liveEdge - 0.8);
      if (!Number.isFinite(target) || target - this.video.currentTime < 1.25) return false;

      this.emit("connecting", "正在追赶直播", "播放器正在回到最新画面");
      this.video.currentTime = target;
      void this.video.play().catch(() => undefined);
      return true;
    } catch {
      return false;
    }
  }

  private scheduleRecovery(reason: string): void {
    if (!this.source || this.reconnectTimer !== undefined) return;
    if (!navigator.onLine) {
      this.handleOffline();
      return;
    }

    this.clearContinuationPreparation();

    const now = Date.now();
    const decision = decideRecoveryLine(
      {
        failuresOnCurrentLine: this.failuresOnCurrentLine,
        lastFailureAt: this.lastFailureAt,
      },
      now,
      this.currentStream?.lineCount ?? 0,
    );
    this.failuresOnCurrentLine = decision.failuresOnCurrentLine;
    this.lastFailureAt = decision.lastFailureAt;
    this.reconnectCount += 1;
    this.lastRecoveryReason = compactReason(reason);
    if (decision.switchLine) {
      this.lineCursor += 1;
      this.lineSwitchCount += 1;
      this.lastRecoveryAction = "切换备用线路";
    } else {
      this.lastRecoveryAction = "刷新当前线路";
    }
    this.attempt += 1;
    const delay = RECONNECT_DELAYS_MS[Math.min(this.attempt - 1, RECONNECT_DELAYS_MS.length - 1)];
    const generation = this.generation;
    this.emit(
      "recovering",
      "连接波动，正在自动恢复",
      delay === 0
        ? `${this.lastRecoveryReason} · ${this.lastRecoveryAction}`
        : `${compactReason(reason)} · ${Math.ceil(delay / 1_000)} 秒后重试`,
    );
    this.reconnectTimer = window.setTimeout(() => {
      this.reconnectTimer = undefined;
      void this.connect(generation);
    }, delay);
  }

  private readonly handleOnline = (): void => {
    if (!this.source) return;
    window.clearTimeout(this.reconnectTimer);
    this.reconnectTimer = undefined;
    void this.connect(this.generation);
  };

  private readonly handleOffline = (): void => {
    if (!this.source) return;
    this.connecting = false;
    window.clearTimeout(this.reconnectTimer);
    this.reconnectTimer = undefined;
    this.emit("error", "网络已断开", "网络恢复后会自动重新连接");
  };

  private readonly handleVisibilityChange = (): void => {
    if (
      document.visibilityState === "visible" &&
      this.source &&
      !this.userPaused &&
      Date.now() - this.lastAdvanceAt >= STALL_LIMIT_MS
    ) {
      this.scheduleRecovery("应用恢复后未检测到新画面");
    }
  };

  private readonly collectStats = (): void => {
    const stream = this.currentStream;
    if (!stream) {
      this.emitEmptyStats();
      return;
    }

    const now = performance.now();
    const live = stream.isLive && Boolean(stream.url);
    const replay = stream.isReplay && Boolean(stream.url);
    const playing = live || replay;
    const elapsedMs = Math.max(now - this.lastStatsAt, 1);
    const quality = this.video.getVideoPlaybackQuality?.();
    const webkitQuality = this.video as HTMLVideoElement & {
      webkitDecodedFrameCount?: number;
      webkitDroppedFrameCount?: number;
    };
    const totalFrames = quality?.totalVideoFrames ?? webkitQuality.webkitDecodedFrameCount ?? null;
    const droppedFrames = quality?.droppedVideoFrames ?? webkitQuality.webkitDroppedFrameCount ?? null;

    let frameRate: number | null = null;
    if (
      totalFrames !== null &&
      this.lastFrameCount !== undefined &&
      totalFrames >= this.lastFrameCount &&
      totalFrames > this.lastFrameCount
    ) {
      frameRate = ((totalFrames - this.lastFrameCount) * 1_000) / elapsedMs;
      this.lastFrameAdvanceAt = Date.now();
      this.softRecoveryAt = 0;
    }

    const width = this.video.videoWidth;
    const height = this.video.videoHeight;
    const currentBuffer = playing ? bufferedAhead(this.video) : null;
    const stabilityNow = Date.now();
    this.recentBilibiliBufferEvents = this.recentBilibiliBufferEvents
      .filter((timestamp) => stabilityNow - timestamp <= BILIBILI_BUFFER_WINDOW_MS);
    if (
      stream.platform === "bilibili"
      && stream.isLive
      && !this.preferStableBilibiliQuality
      && bilibiliQn(stream.qualityLabel) > 400
      && shouldPreferStableBilibiliQuality({
        recentBufferEvents: this.recentBilibiliBufferEvents.length,
        bufferedSeconds: currentBuffer,
        droppedFrames,
        totalFrames,
      })
    ) {
      this.scheduleBilibiliStableQuality();
    }
    this.onStats({
      active: true,
      live,
      replay,
      platform: stream.platform,
      platformLabel: stream.platformLabel,
      sourceUrl: stream.sourceUrl,
      roomId: stream.roomId,
      anchor: stream.anchor,
      title: stream.title,
      avatarUrl: stream.avatarUrl,
      qualityLabel: stream.qualityLabel,
      nominalBitrateKbps: stream.bitrate,
      measuredBitrateKbps: playing ? this.measuredBitrateKbps : null,
      resolution: playing && width > 0 && height > 0 ? `${width} × ${height}` : "",
      frameRate: playing ? frameRate : null,
      bufferedSeconds: currentBuffer,
      droppedFrames: playing ? droppedFrames : null,
      totalFrames: playing ? totalFrames : null,
      lineLabel:
        replay
          ? "录像回放"
          : live && stream.lineCount > 0
          ? [stream.lineName, `${stream.lineIndex + 1}/${stream.lineCount}`]
              .filter(Boolean)
              .join(" · ")
          : "",
      connectionHealth: this.connectionHealthLabel(),
    });

    if (totalFrames !== null) this.lastFrameCount = totalFrames;
    this.lastStatsAt = now;
  };

  private emitEmptyStats(): void {
    this.onStats({
      active: false,
      live: false,
      replay: false,
      platform: "",
      platformLabel: "",
      sourceUrl: "",
      roomId: "",
      anchor: "",
      title: "",
      avatarUrl: "",
      qualityLabel: "",
      nominalBitrateKbps: 0,
      measuredBitrateKbps: null,
      resolution: "",
      frameRate: null,
      bufferedSeconds: null,
      droppedFrames: null,
      totalFrames: null,
      lineLabel: "",
      connectionHealth: "",
    });
  }

  private connectionHealthLabel(): string {
    if (this.preferStableBilibiliQuality) {
      return [
        "稳定画质",
        `缓冲 ${this.bilibiliBufferEventCount} 次`,
        `降档 ${this.bilibiliQualityFallbackCount} 次`,
        `换线 ${this.lineSwitchCount} 次`,
      ].join(" · ");
    }
    if (this.reconnectCount === 0 && this.continuationCount === 0) {
      return "稳定 · 未发生重连或换线";
    }
    if (this.reconnectCount === 0) {
      return `稳定 · 同线路续接 ${this.continuationCount} 次 · 换线 0 次`;
    }
    return [
      this.lastRecoveryAction,
      this.continuationCount > 0 ? `同线路续接 ${this.continuationCount} 次` : "",
      `重连 ${this.reconnectCount} 次`,
      `换线 ${this.lineSwitchCount} 次`,
      this.lastRecoveryReason,
    ].filter(Boolean).join(" · ");
  }

  private resetRecoveryHistory(): void {
    this.failuresOnCurrentLine = 0;
    this.lastFailureAt = 0;
    this.reconnectCount = 0;
    this.lineSwitchCount = 0;
    this.continuationCount = 0;
    this.lastRecoveryReason = "";
    this.lastRecoveryAction = "";
    this.preparedContinuation = undefined;
    this.silentContinuation = false;
    this.preferStableBilibiliQuality = false;
    this.recentBilibiliBufferEvents = [];
    this.lastBilibiliBufferEventAt = 0;
    this.bilibiliBufferEventCount = 0;
    this.bilibiliQualityFallbackCount = 0;
  }

  private scheduleBilibiliStableQuality(): void {
    if (this.preferStableBilibiliQuality || this.reconnectTimer !== undefined) return;
    this.preferStableBilibiliQuality = true;
    this.bilibiliQualityFallbackCount += 1;
    this.reconnectCount += 1;
    this.lastRecoveryAction = "切换稳定画质";
    this.lastRecoveryReason = "原画连续缓冲或解码掉帧过高";
    const generation = this.generation;
    this.emit(
      "recovering",
      "原画负载较高，正在切换稳定画质",
      "保持当前 CDN，优先请求蓝光 QN 400",
    );
    this.reconnectTimer = window.setTimeout(() => {
      this.reconnectTimer = undefined;
      void this.connect(generation);
    }, 0);
  }

  private clearContinuationPreparation(): void {
    window.clearTimeout(this.continuationPrefetchTimer);
    this.continuationPrefetchTimer = undefined;
    this.preparedContinuation = undefined;
  }

  private resetStatsSample(): void {
    this.lastFrameCount = undefined;
    this.lastStatsAt = performance.now();
    this.lastFrameAdvanceAt = Date.now();
    this.softRecoveryAt = 0;
    this.measuredBitrateKbps = null;
    this.bitrateSamples = [];
  }

  private resetMedia(): void {
    this.suppressMediaEventsUntil = Date.now() + 1_000;
    this.destroyPlayers();
    this.video.pause();
    this.video.loop = false;
    this.video.removeAttribute("src");
    this.video.load();
    this.lastMediaTime = 0;
    this.lastAdvanceAt = Date.now();
    this.resetStatsSample();
  }

  private destroyHls(): void {
    const hls = this.hls;
    this.hls = undefined;
    this.hlsMediaRecoveryCount = 0;
    hls?.destroy();
  }

  private destroyPlayers(): void {
    this.destroyHls();
    const player = this.mpegtsPlayer;
    this.mpegtsPlayer = undefined;
    if (!player) return;
    try {
      player.unload();
      player.detachMediaElement();
      player.destroy();
    } catch {
      // The media element can already be detached after a WebKit failure.
    }
  }

  private clearTimers(): void {
    window.clearTimeout(this.reconnectTimer);
    window.clearTimeout(this.stableTimer);
    this.clearContinuationPreparation();
    this.reconnectTimer = undefined;
    this.stableTimer = undefined;
  }

  private emit(
    phase: PlaybackPhase,
    headline: string,
    detail: string,
    mutedByPolicy = false,
  ): void {
    this.onState({ phase, headline, detail, mutedByPolicy });
  }
}

function waitForMediaMetadata(video: HTMLVideoElement): Promise<void> {
  if (video.readyState >= HTMLMediaElement.HAVE_METADATA) return Promise.resolve();

  return new Promise<void>((resolve, reject) => {
    const cleanup = (): void => {
      video.removeEventListener("loadedmetadata", handleLoaded);
      video.removeEventListener("error", handleError);
    };
    const handleLoaded = (): void => {
      cleanup();
      resolve();
    };
    const handleError = (): void => {
      cleanup();
      reject(new Error(mediaErrorMessage(video.error)));
    };
    video.addEventListener("loadedmetadata", handleLoaded, { once: true });
    video.addEventListener("error", handleError, { once: true });
  });
}

function withTimeout<T>(promise: Promise<T>, timeoutMs: number): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timeout = window.setTimeout(
      () => reject(new Error("建立播放连接超时")),
      timeoutMs,
    );
    promise.then(
      (value) => {
        window.clearTimeout(timeout);
        resolve(value);
      },
      (error: unknown) => {
        window.clearTimeout(timeout);
        reject(error);
      },
    );
  });
}

function isAutoplayBlocked(error: unknown): boolean {
  return error instanceof DOMException && error.name === "NotAllowedError";
}

function isUnsupportedMedia(error: unknown): boolean {
  return error instanceof DOMException && error.name === "NotSupportedError";
}

function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "暂时无法建立播放连接";
}

function platformLabelForSource(source: string): string {
  try {
    const url = new URL(source.includes("://") ? source : `https://${source}`);
    return url.hostname.toLowerCase().endsWith("bilibili.com") ? "Bilibili" : "虎牙";
  } catch {
    return "直播平台";
  }
}

function bilibiliQn(qualityLabel: string): number {
  const match = qualityLabel.match(/\bQN\s+(\d+)\b/i);
  return match ? Number(match[1]) : 0;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function mpegtsErrorMessage(
  errorType: unknown,
  errorDetail: unknown,
  errorInfo: unknown,
): string {
  const headline =
    errorType === mpegts.ErrorTypes.NETWORK_ERROR
      ? "直播数据连接中断"
      : errorType === mpegts.ErrorTypes.MEDIA_ERROR
        ? "直播画面解码中断"
        : "FLV 播放连接中断";
  const detail = typeof errorDetail === "string" ? errorDetail : "";
  const info = isRecord(errorInfo) && typeof errorInfo.msg === "string" ? errorInfo.msg : "";
  const diagnostic = [detail, info].filter(Boolean).join(" · ");
  return diagnostic ? `${headline}（${diagnostic}）` : headline;
}

function mediaErrorMessage(error: MediaError | null): string {
  if (!error) return "播放连接已中断";
  switch (error.code) {
    case MediaError.MEDIA_ERR_ABORTED:
      return "播放请求被中断";
    case MediaError.MEDIA_ERR_NETWORK:
      return "直播数据连接中断";
    case MediaError.MEDIA_ERR_DECODE:
      return "当前画面解码失败";
    case MediaError.MEDIA_ERR_SRC_NOT_SUPPORTED:
      return "当前直播地址无法播放";
    default:
      return "播放连接已中断";
  }
}

function hlsErrorMessage(error: ErrorData): string {
  const status = error.response?.code ? ` · HTTP ${error.response.code}` : "";
  const diagnostic = `（${error.details}${status}）`;
  switch (error.type) {
    case ErrorTypes.NETWORK_ERROR:
      return `直播分片连接中断${diagnostic}`;
    case ErrorTypes.MEDIA_ERROR:
      return `直播画面解码中断${diagnostic}`;
    case ErrorTypes.MUX_ERROR:
      return `直播分片格式异常${diagnostic}`;
    default:
      return error.reason?.trim() || error.error?.message || `HLS 播放连接中断${diagnostic}`;
  }
}

function compactReason(reason: string): string {
  const cleaned = reason.replace(/^Error:\s*/i, "").trim();
  return cleaned.length > 46 ? `${cleaned.slice(0, 46)}…` : cleaned;
}

function bufferedAhead(video: HTMLVideoElement): number | null {
  const currentTime = video.currentTime;
  for (let index = 0; index < video.buffered.length; index += 1) {
    const start = video.buffered.start(index);
    const end = video.buffered.end(index);
    if (currentTime >= start - 0.05 && currentTime <= end + 0.05) {
      return Math.max(0, end - currentTime);
    }
  }
  return video.readyState > HTMLMediaElement.HAVE_NOTHING ? 0 : null;
}
