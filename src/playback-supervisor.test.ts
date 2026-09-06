import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { PlaybackSupervisor, type PlaybackStats } from "./playback-supervisor";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("mpegts.js", () => ({ default: {} }));
vi.mock("hls.js", () => ({ default: {} }));

let supervisor: PlaybackSupervisor;
let samples: PlaybackStats[];
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
  vi.stubGlobal("window", Object.assign(new EventTarget(), {
    setInterval, clearInterval, setTimeout, clearTimeout,
  }));
  vi.stubGlobal("document", Object.assign(new EventTarget(), { hidden: false }));
  vi.stubGlobal("navigator", { onLine: true });
  vi.stubGlobal("HTMLMediaElement", { HAVE_NOTHING: 0 });
  const video = Object.assign(new EventTarget(), {
    pause: vi.fn(), load: vi.fn(), removeAttribute: vi.fn(),
    currentTime: 0, videoWidth: 1920, videoHeight: 1080,
    buffered: { length: 0 }, getVideoPlaybackQuality: () => ({ totalVideoFrames: 100, droppedVideoFrames: 0 }),
  }) as unknown as HTMLVideoElement;
  samples = [];
  supervisor = new PlaybackSupervisor(video, () => {}, stats => samples.push(stats));
  vi.spyOn(supervisor as unknown as { playUrl: () => Promise<void> }, "playUrl").mockResolvedValue();
  vi.mocked(invoke).mockResolvedValue(stream());
});
afterEach(() => { supervisor.destroy(); vi.unstubAllGlobals(); vi.clearAllMocks(); });

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
