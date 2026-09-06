import { describe, expect, it, vi } from "vitest";
import { FavoriteLiveStatus } from "./favorite-live-status";

const rooms = Array.from({ length: 8 }, (_, i) => ({ id: String(i), source: `room-${i}` }));

describe("favorite live status", () => {
  it("refreshes expired status and removes live badges after errors or going offline", async () => {
    let now = 0;
    const query = vi.fn().mockResolvedValueOnce(true).mockRejectedValueOnce(new Error("offline"))
      .mockResolvedValueOnce(true).mockResolvedValueOnce(false);
    const status = new FavoriteLiveStatus(query, () => {}, () => now);
    await status.refresh([rooms[0]]);
    expect(status.isLive("0")).toBe(true);
    await status.refresh([rooms[0]]);
    expect(query).toHaveBeenCalledTimes(1);
    now = 60_000;
    expect(status.isLive("0")).toBe(false);
    await status.refresh([rooms[0]]);
    expect(status.isLive("0")).toBe(false);
    now += 60_000;
    await status.refresh([rooms[0]]);
    expect(status.isLive("0")).toBe(true);
    now += 60_000;
    await status.refresh([rooms[0]]);
    expect(status.isLive("0")).toBe(false);
  });

  it("limits concurrency and deduplicates overlapping refreshes", async () => {
    let active = 0;
    let peak = 0;
    const query = vi.fn(async () => {
      active++;
      peak = Math.max(peak, active);
      await new Promise(resolve => setTimeout(resolve, 1));
      active--;
      return true;
    });
    const status = new FavoriteLiveStatus(query, () => {});
    await Promise.all([status.refresh(rooms), status.refresh(rooms)]);
    expect(peak).toBe(3);
    expect(query).toHaveBeenCalledTimes(8);
    expect(rooms.every(room => status.isLive(room.id))).toBe(true);
    await status.refresh([]);
    expect(status.isLive("0")).toBe(false);
  });
});
