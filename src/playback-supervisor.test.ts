import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { PlaybackSupervisor, type PlaybackStats } from "./playback-supervisor";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("mpegts.js", () => ({ default: {} }));
vi.mock("hls.js", async (importOriginal) => ({
  ...await importOriginal<typeof import("hls.js")>(), default: {},
}));

let supervisor: PlaybackSupervisor;
let samples: PlaybackStats[];
let video: HTMLVideoElement;
let ahead: number;
const stream = (qn = 10000, line = 0) => ({
  platform: "bilibili", platformLabel: "Bilibili", sourceUrl: "https://live.bilibili.com/4420009",
  roomId: "4420009", anchor: "主播", title: "直播", avatarUrl: "",
  url: "https://test.bilivideo.com/live.flv", isLive: true, isReplay: false,
  format: "flv", bitrate: 0, qualityLabel: `QN ${qn}`,
  qualityOptions: [{ value: 0, label: "自动" }, { value: 400, label: "蓝光" }],
  lineIndex: line, lineCount: 2, lineName: `cdn-${line}`,
  lineOptions: [{ index: 0, label: "线路 1" }, { index: 1, label: "线路 2" }],
});
const settle = async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); };

beforeEach(() => {
  vi.useFakeTimers();
  vi.stubGlobal("window", Object.assign(new EventTarget(), {
    setInterval, clearInterval, setTimeout, clearTimeout,
  }));
  vi.stubGlobal("document", Object.assign(new EventTarget(), { hidden: false }));
  vi.stubGlobal("navigator", { onLine: true });
  vi.stubGlobal("HTMLMediaElement", { HAVE_NOTHING: 0 });
  ahead = 0;
  video = Object.assign(new EventTarget(), {
    pause: vi.fn(), load: vi.fn(), removeAttribute: vi.fn(),
    play: vi.fn().mockResolvedValue(undefined), src: "blob:test", readyState: 2,
    currentTime: 0, videoWidth: 1920, videoHeight: 1080,
    buffered: { length: 1, start: () => 0, end: () => ahead },
    getVideoPlaybackQuality: () => ({ totalVideoFrames: 100, droppedVideoFrames: 0 }),
  }) as unknown as HTMLVideoElement;
  samples = [];
  supervisor = new PlaybackSupervisor(video, () => {}, stats => samples.push(stats));
  vi.spyOn(supervisor as unknown as { playUrl: () => Promise<void> }, "playUrl").mockResolvedValue();
  vi.mocked(invoke).mockResolvedValue(stream());
});
afterEach(() => { supervisor.destroy(); vi.unstubAllGlobals(); vi.clearAllMocks(); vi.useRealTimers(); });

it("starts resolving immediately and clears old measurements while switching quality", async () => {
  supervisor.start("https://live.bilibili.com/4420009");
  expect(invoke).toHaveBeenCalledOnce();
  expect(samples[samples.length - 1]).toMatchObject({ active: false, connecting: true });
  await settle();
  expect(samples[samples.length - 1]).toMatchObject({ active: true, connecting: false, qualityLabel: "QN 10000" });
  let finish!: (value: unknown) => void;
  vi.mocked(invoke).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  supervisor.selectQuality(400);
  expect(samples[samples.length - 1]).toMatchObject({ active: false, connecting: true, resolution: "" });
  finish(stream(400));
  await settle();
  expect(samples[samples.length - 1]).toMatchObject({ connecting: false, qualityLabel: "QN 400", selectedQualityValue: 400 });
  const pending = samples.find(sample => sample.active && sample.connecting);
  expect(pending).toMatchObject({ resolution: "", frameRate: null, bufferedSeconds: null });
});

it("publishes the server applied line immediately after switching", async () => {
  supervisor.start("https://live.bilibili.com/4420009");
  await settle();
  vi.mocked(invoke).mockResolvedValueOnce(stream(10000, 1));
  supervisor.selectLine(1);
  await settle();
  expect(samples[samples.length - 1]).toMatchObject({ connecting: false, selectedLineIndex: 1, lineLabel: "cdn-1 · 2/2" });
});

it("keeps downloading through an underflow and resumes only with a usable reserve", async () => {
  supervisor.start(stream().sourceUrl);
  await settle();
  vi.advanceTimersByTime(1_001);
  video.dispatchEvent(new Event("waiting"));
  expect(video.pause).toHaveBeenCalled();
  ahead = 0.3;
  vi.advanceTimersByTime(1_000);
  expect(video.play).not.toHaveBeenCalled();
  ahead = 4;
  vi.advanceTimersByTime(250);
  expect(video.play).toHaveBeenCalledOnce();
  expect(invoke).toHaveBeenCalledOnce();
});

it("does not interrupt healthy playback for a network stalled event", async () => {
  supervisor.start(stream().sourceUrl);
  await settle();
  vi.mocked(video.pause).mockClear();
  ahead = 5;
  vi.advanceTimersByTime(1_001);
  video.dispatchEvent(new Event("stalled"));
  vi.advanceTimersByTime(1_000);
  expect(video.pause).not.toHaveBeenCalled();
  expect(video.play).not.toHaveBeenCalled();
});

it("does not resume a refill after the user pauses or stops", async () => {
  supervisor.start(stream().sourceUrl);
  await settle();
  vi.advanceTimersByTime(1_001);
  video.dispatchEvent(new Event("waiting"));
  await supervisor.togglePause();
  ahead = 5;
  vi.advanceTimersByTime(1_000);
  expect(video.play).not.toHaveBeenCalled();
  supervisor.stop();
  vi.advanceTimersByTime(15_000);
  expect(video.play).not.toHaveBeenCalled();
});

it("recovers a dead connection instead of waiting indefinitely for refill", async () => {
  supervisor.start(stream().sourceUrl);
  await settle();
  vi.advanceTimersByTime(1_001);
  video.dispatchEvent(new Event("waiting"));
  await vi.advanceTimersByTimeAsync(12_100);
  expect(invoke).toHaveBeenCalledTimes(2);
  expect(video.play).not.toHaveBeenCalled();
});

it("resumes from downloaded frames beyond a timestamp gap without reconnecting", async () => {
  supervisor.start(stream().sourceUrl);
  await settle();
  video.currentTime = 5;
  Object.assign(video, { buffered: {
    length: 2, start: (i: number) => [0, 5.2][i], end: (i: number) => [5, 10][i],
  } });
  vi.advanceTimersByTime(1_001);
  video.dispatchEvent(new Event("waiting"));
  vi.advanceTimersByTime(250);
  expect(video.currentTime).toBeCloseTo(5.25);
  expect(video.play).toHaveBeenCalledOnce();
  expect(invoke).toHaveBeenCalledOnce();
});

it("does not mistake separate short ranges for four seconds of playable buffer", async () => {
  supervisor.start(stream().sourceUrl);
  await settle();
  video.currentTime = 5;
  Object.assign(video, { buffered: {
    length: 3, start: (i: number) => [0, 5.2, 9][i], end: (i: number) => [5, 7, 11][i],
  } });
  vi.advanceTimersByTime(1_001);
  video.dispatchEvent(new Event("waiting"));
  vi.advanceTimersByTime(250);
  expect(video.currentTime).toBe(5);
  expect(video.play).not.toHaveBeenCalled();
  vi.advanceTimersByTime(11_750);
  expect(video.currentTime).toBeCloseTo(5.25);
  expect(video.play).toHaveBeenCalledOnce();
  expect(invoke).toHaveBeenCalledOnce();
});

it("uses actual downloaded ranges for stall recovery, not a broader seekable timeline", async () => {
  supervisor.start(stream().sourceUrl);
  await settle();
  video.currentTime = 5;
  Object.assign(video, {
    buffered: { length: 2, start: (i: number) => [0, 18][i], end: (i: number) => [5, 20][i] },
    seekable: { length: 1, start: () => 0, end: () => 20 },
  });
  vi.advanceTimersByTime(15_000);
  expect(video.currentTime).toBeCloseTo(18.05);
  expect(video.play).toHaveBeenCalledOnce();
});
