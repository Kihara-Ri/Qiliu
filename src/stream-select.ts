/** App-styled single selection; the popover stays above the scrolling panel. */
export class StreamSelect extends EventTarget {
  value = "";
  private options: Array<{ value: number; label: string }> = [];
  private readonly menu = document.createElement("div");
  private active = 0;

  constructor(private readonly trigger: HTMLButtonElement) {
    super();
    this.menu.id = `${trigger.id}-options`;
    this.menu.className = "stream-select-menu";
    this.menu.setAttribute("popover", "auto");
    this.menu.setAttribute("role", "listbox");
    this.menu.setAttribute("aria-label", trigger.getAttribute("aria-label") || "选项");
    trigger.setAttribute("aria-haspopup", "listbox");
    trigger.setAttribute("aria-controls", this.menu.id);
    trigger.setAttribute("aria-expanded", "false");
    trigger.after(this.menu);
    trigger.addEventListener("click", () => this.open());
    trigger.addEventListener("keydown", (event) => {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        this.open();
      }
    });
    this.menu.addEventListener("toggle", () => {
      trigger.setAttribute("aria-expanded", String(this.menu.matches(":popover-open")));
    });
    this.menu.addEventListener("keydown", (event) => {
      const keys = ["ArrowDown", "ArrowUp", "Home", "End", "Escape", "Tab"];
      if (!keys.includes(event.key)) return;
      if (event.key === "Tab" || event.key === "Escape") {
        this.close();
        if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); }
        return;
      }
      event.preventDefault();
      this.active = event.key === "Home" ? 0 : event.key === "End" ? this.options.length - 1
        : (this.active + (event.key === "ArrowDown" ? 1 : -1) + this.options.length) % this.options.length;
      (this.menu.children[this.active] as HTMLElement)?.focus();
    });
    window.addEventListener("resize", () => this.close(false));
  }

  set disabled(disabled: boolean) {
    this.trigger.disabled = disabled;
    if (disabled) this.close(false);
  }

  update(options: Array<{ value: number; label: string }>, selected: number): void {
    const changed = JSON.stringify(options) !== JSON.stringify(this.options);
    const value = String(options.find((option) => option.value === selected)?.value ?? options[0]?.value ?? "");
    if (!changed && value === this.value) return;
    this.options = options;
    this.value = value;
    this.trigger.textContent = options.find((option) => String(option.value) === value)?.label || "等待播放";
    this.menu.replaceChildren(...options.map((option) => {
      const button = document.createElement("button");
      button.type = "button";
      button.tabIndex = -1;
      button.setAttribute("role", "option");
      button.setAttribute("aria-selected", String(String(option.value) === value));
      button.textContent = option.label;
      button.addEventListener("click", () => {
        this.close();
        if (this.value === String(option.value)) return;
        this.update(this.options, option.value);
        this.dispatchEvent(new Event("change"));
      });
      return button;
    }));
  }

  private open(): void {
    if (this.trigger.disabled || !this.options.length) return;
    const rect = this.trigger.getBoundingClientRect();
    const height = Math.min(256, this.options.length * 36 + 8);
    this.menu.style.width = `${Math.min(Math.max(rect.width, 220), window.innerWidth - 24)}px`;
    this.menu.style.left = `${Math.max(12, Math.min(rect.left, window.innerWidth - Math.max(rect.width, 220) - 12))}px`;
    this.menu.style.top = `${rect.bottom + height + 12 < window.innerHeight ? rect.bottom + 6 : Math.max(12, rect.top - height - 6)}px`;
    this.menu.showPopover();
    this.trigger.setAttribute("aria-expanded", "true");
    this.active = Math.max(0, this.options.findIndex((option) => String(option.value) === this.value));
    (this.menu.children[this.active] as HTMLElement)?.focus();
  }

  close(focus = true): void {
    if (!this.menu.matches(":popover-open")) return;
    this.menu.hidePopover();
    this.trigger.setAttribute("aria-expanded", "false");
    if (focus) this.trigger.focus();
  }
}
