export const LINE_FAILURE_WINDOW_MS = 30_000;
export const SAME_LINE_FAILURES_BEFORE_SWITCH = 2;
export const BILIBILI_BUFFER_WINDOW_MS = 90_000;
export const BILIBILI_BUFFER_EVENT_COOLDOWN_MS = 4_000;
export const BILIBILI_BUFFER_EVENTS_BEFORE_FALLBACK = 3;
export const BILIBILI_DROPPED_FRAME_FALLBACK_RATIO = 0.08;

export interface RecoveryHistory {
  failuresOnCurrentLine: number;
  lastFailureAt: number;
}

export interface RecoveryDecision extends RecoveryHistory {
  switchLine: boolean;
}

/**
 * An isolated failure refreshes the signed URL on the same CDN. A line is only
 * rotated when the refreshed connection fails again within a short window.
 */
export function decideRecoveryLine(
  history: RecoveryHistory,
  now: number,
  lineCount: number,
): RecoveryDecision {
  const stillInFailureWindow = history.lastFailureAt > 0
    && now - history.lastFailureAt <= LINE_FAILURE_WINDOW_MS;
  const failures = (stillInFailureWindow ? history.failuresOnCurrentLine : 0) + 1;
  const switchLine = lineCount > 1 && failures >= SAME_LINE_FAILURES_BEFORE_SWITCH;

  return {
    switchLine,
    failuresOnCurrentLine: switchLine ? 0 : failures,
    lastFailureAt: now,
  };
}

export function recordBilibiliBufferEvent(
  recentEvents: readonly number[],
  lastEventAt: number,
  now: number,
): { accepted: boolean; recentEvents: number[]; lastEventAt: number } {
  const retained = recentEvents.filter((timestamp) => now - timestamp <= BILIBILI_BUFFER_WINDOW_MS);
  if (lastEventAt > 0 && now - lastEventAt < BILIBILI_BUFFER_EVENT_COOLDOWN_MS) {
    return { accepted: false, recentEvents: retained, lastEventAt };
  }
  return { accepted: true, recentEvents: [...retained, now], lastEventAt: now };
}

export function shouldPreferStableBilibiliQuality(input: {
  recentBufferEvents: number;
  bufferedSeconds: number | null;
  droppedFrames: number | null;
  totalFrames: number | null;
}): boolean {
  if (input.recentBufferEvents >= BILIBILI_BUFFER_EVENTS_BEFORE_FALLBACK) return true;
  if (
    input.recentBufferEvents === 0
    || input.bufferedSeconds === null
    || input.bufferedSeconds > 3
    || input.droppedFrames === null
    || input.totalFrames === null
    || input.totalFrames < 1_500
  ) {
    return false;
  }
  return input.droppedFrames / input.totalFrames >= BILIBILI_DROPPED_FRAME_FALLBACK_RATIO;
}
