/** Session-only status cache: failed requests never retain a stale live badge. */
export class FavoriteLiveStatus {
  private entries = new Map<string, { live: boolean | null; checkedAt: number }>();
  private pending = new Set<string>();
  private active = 0;

  constructor(
    private query: (source: string) => Promise<boolean>,
    private changed: () => void,
    private now: () => number = Date.now,
  ) {}

  isLive(id: string): boolean {
    const entry = this.entries.get(id);
    return entry?.live === true && this.now() - entry.checkedAt < 60_000;
  }

  async refresh(favorites: readonly { id: string; source: string }[]): Promise<void> {
    const ids = new Set(favorites.map(item => item.id));
    for (const id of this.entries.keys()) if (!ids.has(id)) this.entries.delete(id);
    const queue = favorites.filter(item => !this.pending.has(item.id)
      && (!this.entries.has(item.id) || this.now() - this.entries.get(item.id)!.checkedAt >= 60_000));
    const worker = async () => {
      while (queue.length && this.active < 3) {
        const item = queue.shift()!;
        const cached = this.entries.get(item.id);
        if (this.pending.has(item.id) || (cached && this.now() - cached.checkedAt < 60_000)) continue;
        this.pending.add(item.id);
        this.active++;
        try {
          const live = await this.query(item.source);
          this.entries.set(item.id, { live, checkedAt: this.now() });
        } catch {
          this.entries.set(item.id, { live: null, checkedAt: this.now() });
        } finally {
          this.active--;
          this.pending.delete(item.id);
          this.changed();
        }
      }
    };
    this.changed();
    await Promise.all([worker(), worker(), worker()]);
  }
}
