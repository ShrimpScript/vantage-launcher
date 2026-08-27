/**
 * Version picker.
 *
 * This started as a styled native <select>, on the argument that a hand-rolled listbox is
 * usually worse for keyboard and screen-reader users. Seeing it open settled it: the GTK
 * popup is a light-grey system widget in the middle of a dark launcher, and it looks broken.
 *
 * So it is custom — but it implements the full listbox keyboard contract rather than a div
 * with a click handler: Up/Down/Home/End, Enter/Space, Escape, typeahead, roving
 * aria-activedescendant, and focus returned to the trigger on close.
 */

export type PickerItem = { id: string; group: string; meta: string };

export class Picker {
  private open = false;
  private active = 0;
  private typed = "";
  private typedAt = 0;
  private list: HTMLDivElement;
  private onPick: (id: string) => void = () => {};

  constructor(
    private trigger: HTMLButtonElement,
    private label: HTMLElement,
    private items: PickerItem[],
    private valueRef: { value: string },
  ) {
    this.list = document.createElement("div");
    this.list.className = "picker";
    this.list.setAttribute("role", "listbox");
    this.list.hidden = true;
    trigger.insertAdjacentElement("afterend", this.list);

    trigger.setAttribute("aria-haspopup", "listbox");
    trigger.setAttribute("aria-expanded", "false");

    trigger.addEventListener("click", () => this.toggle());
    trigger.addEventListener("keydown", (e) => {
      if (["ArrowDown", "ArrowUp", "Enter", " "].includes(e.key)) {
        e.preventDefault();
        this.toggle(true);
      }
    });
    this.list.addEventListener("keydown", (e) => this.keydown(e));
    document.addEventListener("pointerdown", (e) => {
      if (this.open && !this.list.contains(e.target as Node) && e.target !== trigger) this.close();
    });
  }

  onChange(fn: (id: string) => void) {
    this.onPick = fn;
  }

  /** Reflect a selection made elsewhere (e.g. clicking an installed-version card). */
  select(id: string) {
    this.valueRef.value = id;
    this.label.textContent = id;
    const i = this.items.findIndex((x) => x.id === id);
    if (i >= 0) this.active = i;
    this.render();
  }

  setItems(items: PickerItem[], value: string) {
    this.items = items;
    this.valueRef.value = value;
    this.active = Math.max(0, items.findIndex((i) => i.id === value));
    this.render();
    this.label.textContent = value;
  }

  private render() {
    const frag = document.createDocumentFragment();
    let group = "";
    this.items.forEach((item, i) => {
      if (item.group !== group) {
        group = item.group;
        const h = document.createElement("div");
        h.className = "pgroup lab";
        h.textContent = group;
        frag.append(h);
      }
      const row = document.createElement("div");
      row.className = "popt";
      row.id = `popt-${i}`;
      row.setAttribute("role", "option");
      row.setAttribute("aria-selected", String(item.id === this.valueRef.value));
      row.dataset.index = String(i);

      const name = document.createElement("span");
      name.className = "pname";
      name.textContent = item.id;
      const meta = document.createElement("span");
      meta.className = "pmeta m";
      meta.textContent = item.meta;
      row.append(name, meta);
      row.addEventListener("click", () => this.commit(i));
      row.addEventListener("pointermove", () => this.setActive(i));
      frag.append(row);
    });
    this.list.replaceChildren(frag);
    this.paintActive();
  }

  private rows() {
    return Array.from(this.list.querySelectorAll<HTMLElement>(".popt"));
  }

  private paintActive() {
    this.rows().forEach((r) => r.classList.toggle("on", Number(r.dataset.index) === this.active));
    const el = this.list.querySelector<HTMLElement>(`#popt-${this.active}`);
    if (el) {
      this.list.setAttribute("aria-activedescendant", el.id);
      el.scrollIntoView({ block: "nearest" });
    }
  }

  private setActive(i: number) {
    this.active = Math.min(Math.max(i, 0), this.items.length - 1);
    this.paintActive();
  }

  private toggle(force?: boolean) {
    force || !this.open ? this.show() : this.close();
  }

  private show() {
    this.open = true;
    this.list.hidden = false;
    this.trigger.setAttribute("aria-expanded", "true");
    this.list.tabIndex = -1;
    this.list.focus();
    this.paintActive();
  }

  close() {
    if (!this.open) return;
    this.open = false;
    this.list.hidden = true;
    this.trigger.setAttribute("aria-expanded", "false");
    this.trigger.focus();
  }

  private commit(i: number) {
    const item = this.items[i];
    if (!item) return;
    this.valueRef.value = item.id;
    this.label.textContent = item.id;
    this.rows().forEach((r) =>
      r.setAttribute("aria-selected", String(Number(r.dataset.index) === i)),
    );
    this.close();
    this.onPick(item.id);
  }

  private keydown(e: KeyboardEvent) {
    switch (e.key) {
      case "ArrowDown": e.preventDefault(); this.setActive(this.active + 1); return;
      case "ArrowUp":   e.preventDefault(); this.setActive(this.active - 1); return;
      case "Home":      e.preventDefault(); this.setActive(0); return;
      case "End":       e.preventDefault(); this.setActive(this.items.length - 1); return;
      case "Enter":
      case " ":         e.preventDefault(); this.commit(this.active); return;
      case "Escape":    e.preventDefault(); this.close(); return;
      case "Tab":       this.close(); return;
    }
    // Typeahead: "1.21" jumps to the first match, resetting after a second of no typing.
    if (e.key.length === 1) {
      const now = Date.now();
      this.typed = now - this.typedAt > 1000 ? e.key : this.typed + e.key;
      this.typedAt = now;
      const hit = this.items.findIndex((i) => i.id.toLowerCase().startsWith(this.typed.toLowerCase()));
      if (hit >= 0) this.setActive(hit);
    }
  }
}
