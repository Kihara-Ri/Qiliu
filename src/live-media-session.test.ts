import { afterEach, expect, it, vi } from "vitest";
import { LiveMediaSession } from "./live-media-session";

afterEach(() => vi.unstubAllGlobals());
it("publishes a broadcast with cover and subtitle, with only play and pause enabled", () => {
  const handlers = new Map();
  const session = { metadata: null, playbackState: "none", setPositionState: vi.fn(),
    setActionHandler: (action: string, handler: unknown) => handlers.set(action, handler) };
  vi.stubGlobal("navigator", { mediaSession: session });
  vi.stubGlobal("MediaMetadata", class { constructor(data: object) { Object.assign(this, data); } });
  const play = vi.fn(), pause = vi.fn();
  const controls = new LiveMediaSession(play, pause);
  controls.update({ title: "直播间", anchor: "主播", coverUrl: "http://image/cover.jpg", avatarUrl: "https://image/avatar.jpg" }, "playing");
  expect(session.metadata).toMatchObject({ title: "栖流 · Qiliu", artist: "直播间 · 主播", artwork: [{ src: "https://image/cover.jpg" }, { src: "https://image/avatar.jpg" }] });
  expect(session.setPositionState).toHaveBeenLastCalledWith({ duration: Infinity, position: 0, playbackRate: 1 });
  for (const action of ["seekto", "seekforward", "seekbackward", "nexttrack", "previoustrack"]) expect(handlers.get(action)).toBeNull();
  handlers.get("pause")();
  handlers.get("play")();
  expect(pause).toHaveBeenCalledOnce();
  expect(play).toHaveBeenCalledOnce();
  controls.update({ title: "新房间", anchor: "新主播", coverUrl: "", avatarUrl: "https://image/avatar.jpg" }, "paused");
  expect(session.playbackState).toBe("paused");
  expect(session.metadata).toMatchObject({ artist: "新房间 · 新主播", artwork: [{ src: "https://image/avatar.jpg" }] });
  controls.update(null, "none");
  expect(session.metadata).toBeNull();
  expect(session.setPositionState).toHaveBeenLastCalledWith({});
});

it("does not refetch artwork when only playback statistics change", () => {
  const session = { metadata: null, playbackState: "none", setPositionState: vi.fn(), setActionHandler: vi.fn() };
  const metadata = vi.fn();
  vi.stubGlobal("navigator", { mediaSession: session });
  vi.stubGlobal("MediaMetadata", class { constructor(data: object) { metadata(data); Object.assign(this, data); } });
  const controls = new LiveMediaSession(() => {}, () => {});
  const room = { title: "直播", anchor: "主播", coverUrl: "https://image/cover", avatarUrl: "" };
  controls.update({ ...room, frames: 100 } as typeof room, "playing");
  controls.update({ ...room, frames: 200 } as typeof room, "playing");
  expect(metadata).toHaveBeenCalledOnce();
  expect(session.setPositionState).toHaveBeenCalledOnce();
});

it("suppresses the buffer timeline on WebKit versions that reject Infinity", () => {
  const session = { metadata: null, playbackState: "none", setActionHandler: vi.fn(),
    setPositionState: vi.fn((state?: MediaPositionState) => {
      if (state?.duration === Infinity) throw new TypeError("non-finite");
    }) };
  vi.stubGlobal("navigator", { mediaSession: session });
  vi.stubGlobal("MediaMetadata", class { constructor(data: object) { Object.assign(this, data); } });
  const controls = new LiveMediaSession(() => {}, () => {});
  controls.update({ title: "直播", anchor: "主播", coverUrl: "", avatarUrl: "" }, "playing");
  expect(session.setPositionState).toHaveBeenLastCalledWith({ duration: 0, position: 0, playbackRate: 1 });
});
