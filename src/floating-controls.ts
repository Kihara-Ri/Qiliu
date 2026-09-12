type Position = { x: number; y: number };
type Preferences = { active: number; idle: number; settings?: Position; volume?: Position };
const STORAGE_KEY = "qiliu.floating-controls.v1";
const clamp = (n: number, min: number, max: number) => Math.min(max, Math.max(min, n));

export function readControlPreferences(raw: string | null): Preferences {
  const defaults: Preferences = { active: 100, idle: 25 };
  try {
    const value = JSON.parse(raw ?? "null");
    if (!value || typeof value !== "object") return defaults;
    for (const key of ["active", "idle"] as const) {
      if (typeof value[key] === "number" && Number.isFinite(value[key])) {
        defaults[key] = clamp(value[key], key === "active" ? 20 : 0, 100);
      }
    }
    for (const key of ["settings", "volume"] as const) {
      const point = value[key];
      if (point && Number.isFinite(point.x) && Number.isFinite(point.y)) {
        defaults[key] = { x: clamp(point.x, 0, 1), y: clamp(point.y, 0, 1) };
      }
    }
    return defaults;
  } catch { return defaults; }
}

export function setupFloatingControls(
  app: HTMLElement, settings: HTMLButtonElement, volume: HTMLElement, closeSettings: () => void,
): void {
  const get = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
  let preferences = readControlPreferences(localStorage.getItem(STORAGE_KEY));
  const items = [
    { key: "settings" as const, root: settings, handle: settings },
    { key: "volume" as const, root: volume, handle: get<HTMLButtonElement>("volume-toggle") },
  ];
  const bar = get<HTMLElement>("control-edit-bar");
  let editing = false;
  let pointer: { x: number; y: number } | undefined;
  let timer = 0;
  let dragging = false;
  const save = () => localStorage.setItem(STORAGE_KEY, JSON.stringify(preferences));
  const nearControl = () => {
    if (!pointer) return false;
    const { x, y } = pointer;
    const hit = document.elementFromPoint(x, y);
    if (hit?.closest(".settings-panel, .control-edit-bar")) return false;
    return items.some(item => {
      const targets: HTMLElement[] = [item.handle];
      if (item.key === "volume" && item.root.classList.contains("is-expanded")) {
        targets.push(get("volume-slider-shell"));
      }
      return targets.some(target => {
        const rect = target.getBoundingClientRect();
        return x >= rect.left - 28 && x <= rect.right + 28
          && y >= rect.top - 28 && y <= rect.bottom + 28;
      });
    });
  };
  const wake = () => {
    app.classList.add("controls-awake");
    window.clearTimeout(timer);
    timer = window.setTimeout(() => {
      if (!nearControl() && !dragging) app.classList.remove("controls-awake");
    }, 2000);
  };
  const bounds = (handle: HTMLElement) => ({
    width: Math.max(0, app.clientWidth - handle.offsetWidth - 24),
    height: Math.max(0, app.clientHeight - handle.offsetHeight - 68),
  });
  const place = () => {
    for (const item of items) {
      const point = preferences[item.key];
      item.root.classList.toggle("control-positioned", !!point);
      if (!point) { item.root.classList.remove("expand-left"); continue; }
      const area = bounds(item.handle);
      item.root.style.setProperty("--control-x", `${12 + point.x * area.width}px`);
      item.root.style.setProperty("--control-y", `${56 + point.y * area.height}px`);
      item.root.classList.toggle("expand-left", point.x > 0.5);
    }
  };
  const render = () => {
    app.style.setProperty("--control-active-opacity", String(preferences.active / 100));
    app.style.setProperty("--control-idle-opacity", String(preferences.idle / 100));
    for (const kind of ["active", "idle"] as const) {
      get<HTMLInputElement>(`control-${kind}-opacity`).value = String(preferences[kind]);
      get<HTMLOutputElement>(`control-${kind}-output`).value = `${preferences[kind]}%`;
    }
    place();
  };
  const setEditing = (value: boolean) => {
    editing = value;
    app.classList.toggle("controls-editing", value);
    bar.hidden = !value;
    if (value) { closeSettings(); settings.focus(); }
    else get<HTMLButtonElement>("control-edit-done").blur();
    wake();
  };
  get("control-edit").addEventListener("click", () => setEditing(true));
  get("control-edit-done").addEventListener("click", () => setEditing(false));
  get("control-reset").addEventListener("click", () => {
    preferences = { active: 100, idle: 25 }; save(); render(); wake();
  });
  for (const kind of ["active", "idle"] as const) {
    get<HTMLInputElement>(`control-${kind}-opacity`).addEventListener("input", event => {
      preferences[kind] = Number((event.target as HTMLInputElement).value);
      render(); save();
    });
  }
  for (const item of items) {
    let drag: { id: number; x: number; y: number; left: number; top: number } | undefined;
    const move = (left: number, top: number) => {
      const area = bounds(item.handle);
      preferences[item.key] = {
        x: clamp((left - 12) / (area.width || 1), 0, 1),
        y: clamp((top - 56) / (area.height || 1), 0, 1),
      };
      place();
    };
    item.handle.addEventListener("click", event => {
      if (editing) { event.preventDefault(); event.stopImmediatePropagation(); }
    }, true);
    item.handle.addEventListener("pointerdown", event => {
      if (!editing || event.button !== 0) return;
      event.preventDefault();
      const rect = item.handle.getBoundingClientRect();
      const parent = app.getBoundingClientRect();
      drag = { id: event.pointerId, x: event.clientX, y: event.clientY,
        left: rect.left - parent.left, top: rect.top - parent.top };
      dragging = true;
      item.handle.setPointerCapture(event.pointerId);
      item.root.classList.add("is-dragging");
    });
    item.handle.addEventListener("pointermove", event => {
      if (!drag || event.pointerId !== drag.id) return;
      move(drag.left + event.clientX - drag.x, drag.top + event.clientY - drag.y);
    });
    const finish = () => {
      if (!drag) return;
      drag = undefined; dragging = false;
      item.root.classList.remove("is-dragging"); save(); wake();
    };
    item.handle.addEventListener("pointerup", finish);
    item.handle.addEventListener("pointercancel", finish);
    item.handle.addEventListener("lostpointercapture", finish);
    item.handle.addEventListener("keydown", event => {
      if (!editing || !["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(event.key)) return;
      event.preventDefault(); event.stopImmediatePropagation();
      const rect = item.handle.getBoundingClientRect();
      const parent = app.getBoundingClientRect();
      const step = event.shiftKey ? 20 : 4;
      move(rect.left - parent.left + (event.key === "ArrowLeft" ? -step : event.key === "ArrowRight" ? step : 0),
        rect.top - parent.top + (event.key === "ArrowUp" ? -step : event.key === "ArrowDown" ? step : 0));
      save();
    }, true);
  }
  const insidePanel = (target: EventTarget | null) =>
    target instanceof Element && !!target.closest(".settings-panel, .control-edit-bar");
  app.addEventListener("pointermove", event => {
    const moved = !pointer || pointer.x !== event.clientX || pointer.y !== event.clientY;
    pointer = event.pointerType === "touch" ? undefined : { x: event.clientX, y: event.clientY };
    if (moved && !insidePanel(event.target)) wake();
  }, { passive: true });
  app.addEventListener("pointerleave", () => { pointer = undefined; wake(); });
  document.addEventListener("pointerdown", event => {
    app.classList.remove("controls-keyboard");
    if (!insidePanel(event.target)) wake();
  }, { passive: true, capture: true });
  app.addEventListener("focusin", event => { if (!insidePanel(event.target)) wake(); });
  window.addEventListener("blur", () => {
    pointer = undefined;
    app.classList.remove("controls-keyboard", "controls-awake");
    window.clearTimeout(timer);
  });
  // Re-evaluate proximity after the panel closes, even if the mouse has not moved.
  let panelOpen = app.classList.contains("settings-open");
  new MutationObserver(() => {
    const open = app.classList.contains("settings-open");
    if (open !== panelOpen) { panelOpen = open; wake(); }
  }).observe(app, { attributes: true, attributeFilter: ["class"] });
  document.addEventListener("keydown", event => {
    if (event.key === "Tab" || event.key.startsWith("Arrow")) app.classList.add("controls-keyboard");
    if (editing && event.key === "Escape") { setEditing(false); settings.focus(); }
  });
  new ResizeObserver(place).observe(app);
  render(); wake();
}
