/** System media controls describe the broadcast, never the current FLV connection. */
export class LiveMediaSession {
  private readonly session = navigator.mediaSession;
  private metadataKey = "";

  constructor(play: () => void, pause: () => void) {
    if (!this.session) return;
    for (const [action, handler] of [
      ["play", play], ["pause", pause],
      ["seekbackward", null], ["seekforward", null], ["seekto", null],
      ["previoustrack", null], ["nexttrack", null], ["stop", null],
    ] as const) {
      try { this.session.setActionHandler(action, handler); } catch { /* Older WebKit. */ }
    }
  }

  update(metadata: { title: string; anchor: string; coverUrl: string; avatarUrl: string } | null,
    state: MediaSessionPlaybackState): void {
    if (!this.session) return;
    const key = metadata ? JSON.stringify([metadata.title, metadata.anchor, metadata.coverUrl, metadata.avatarUrl]) : "";
    if (key === this.metadataKey && this.session.playbackState === state) return;
    if (key !== this.metadataKey) {
      this.metadataKey = key;
      this.session.metadata = metadata && typeof MediaMetadata !== "undefined"
        ? new MediaMetadata({
          title: "栖流 · Qiliu",
          artist: [metadata.title, metadata.anchor].filter(Boolean).join(" · "),
          artwork: [metadata.coverUrl, metadata.avatarUrl].filter(Boolean)
            .map(src => ({ src: secureArtworkUrl(src) })),
        }) : null;
    }
    this.session.playbackState = state;
    try {
      // Clearing position state alone lets WebKit fall back to video.duration,
      // which MSE extends as fragments arrive. Infinity explicitly marks live.
      this.session.setPositionState(metadata
        ? { duration: Infinity, position: 0, playbackRate: 1 } : {});
    } catch {
      // Older WebKit rejects Infinity. A zero/unknown presentation duration
      // suppresses its finite FLV buffer timeline without inventing an end time.
      try { this.session.setPositionState({ duration: 0, position: 0, playbackRate: 1 }); }
      catch { /* Unsupported API. */ }
    }
  }
}

function secureArtworkUrl(src: string): string {
  if (src.startsWith("//")) return `https:${src}`;
  return src.replace(/^http:\/\//u, "https://");
}
