import { describe, expect, it } from "vitest";
import {
  LINE_FAILURE_WINDOW_MS,
  decideRecoveryLine,
  recordBilibiliBufferEvent,
  shouldPreferStableBilibiliQuality,
  type RecoveryHistory,
} from "./playback-recovery-policy";

const emptyHistory: RecoveryHistory = {
  failuresOnCurrentLine: 0,
  lastFailureAt: 0,
};

describe("playback recovery line policy", () => {
  it("refreshes the same CDN for an isolated failure", () => {
    expect(decideRecoveryLine(emptyHistory, 1_000, 3)).toEqual({
      switchLine: false,
      failuresOnCurrentLine: 1,
      lastFailureAt: 1_000,
    });
  });

  it("rotates only after the refreshed line fails again quickly", () => {
    const first = decideRecoveryLine(emptyHistory, 1_000, 3);
    expect(decideRecoveryLine(first, 5_000, 3)).toMatchObject({
      switchLine: true,
      failuresOnCurrentLine: 0,
    });
  });

  it("does not rotate for periodic isolated failures outside the window", () => {
    const first = decideRecoveryLine(emptyHistory, 1_000, 3);
    expect(decideRecoveryLine(first, 1_000 + LINE_FAILURE_WINDOW_MS + 1, 3))
      .toMatchObject({ switchLine: false, failuresOnCurrentLine: 1 });
  });

  it("never claims a line switch when no alternative exists", () => {
    const first = decideRecoveryLine(emptyHistory, 1_000, 1);
    expect(decideRecoveryLine(first, 2_000, 1)).toMatchObject({ switchLine: false });
  });
});

describe("Bilibili stability quality policy", () => {
  it("deduplicates waiting and stalled events from the same underflow", () => {
    const first = recordBilibiliBufferEvent([], 0, 10_000);
    const duplicate = recordBilibiliBufferEvent(first.recentEvents, first.lastEventAt, 12_000);
    expect(first.accepted).toBe(true);
    expect(duplicate).toEqual({
      accepted: false,
      recentEvents: [10_000],
      lastEventAt: 10_000,
    });
  });

  it("falls back after three real underflows in the rolling window", () => {
    expect(shouldPreferStableBilibiliQuality({
      recentBufferEvents: 3,
      bufferedSeconds: 7,
      droppedFrames: 0,
      totalFrames: 500,
    })).toBe(true);
  });

  it("falls back after one underflow when decoder pressure is also high", () => {
    expect(shouldPreferStableBilibiliQuality({
      recentBufferEvents: 1,
      bufferedSeconds: 2.5,
      droppedFrames: 180,
      totalFrames: 2_000,
    })).toBe(true);
  });

  it("keeps original quality for an isolated healthy underflow", () => {
    expect(shouldPreferStableBilibiliQuality({
      recentBufferEvents: 1,
      bufferedSeconds: 5,
      droppedFrames: 30,
      totalFrames: 2_000,
    })).toBe(false);
  });
});
