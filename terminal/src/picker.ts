import {
  type Component,
  type Focusable,
  fuzzyFilter,
  Input,
  parseKey,
  visibleWidth,
} from "@earendil-works/pi-tui";
import { fit, singleLine, style } from "./theme.ts";

/** A selectable identity or conversation shown by Picker. */
export interface PickerItem {
  id: string;
  label: string;
  detail?: string;
  badge?: string;
  category?: string;
}

interface PickerOptions {
  title: string;
  items: PickerItem[] | (() => PickerItem[]);
  onSelect: (item: PickerItem) => void;
  onCancel: () => void;
  requestRender: () => void;
  availableRows?: () => number;
}

/** Focusable, fuzzy-searching terminal picker for compact overlays. */
export class Picker implements Component, Focusable {
  focused = false;
  private readonly title: string;
  private readonly items: PickerOptions["items"];
  private readonly input: Input;
  private readonly onSelect: (item: PickerItem) => void;
  private readonly onCancel: () => void;
  private readonly requestRender: () => void;
  private readonly availableRows: () => number;
  private selected = 0;
  private selectedId?: string;

  constructor(options: PickerOptions) {
    this.title = singleLine(options.title);
    this.items = options.items;
    this.onSelect = options.onSelect;
    this.onCancel = options.onCancel;
    this.requestRender = options.requestRender;
    this.availableRows = options.availableRows ?? (() => 20);
    this.input = new Input({
      prompt: "› ",
      placeholder: "Search",
      placeholderStyle: style.secondary,
    });
  }

  private filtered(): PickerItem[] {
    const items = typeof this.items === "function" ? this.items() : this.items;
    const filtered = fuzzyFilter(
      items.map((item) => ({
        ...item,
        label: singleLine(item.label),
        detail: item.detail === undefined ? undefined : singleLine(item.detail),
        badge: item.badge === undefined ? undefined : singleLine(item.badge),
        category:
          item.category === undefined ? undefined : singleLine(item.category),
      })),
      this.input.getValue(),
      (item) =>
        `${item.label} ${item.detail ?? ""} ${item.badge ?? ""} ${item.category ?? ""}`,
    );
    const index = filtered.findIndex((item) => item.id === this.selectedId);
    this.selected = index < 0 ? 0 : index;
    this.selectedId = filtered[this.selected]?.id;
    return filtered;
  }

  handleInput(data: string): void {
    const key = parseKey(data);
    const filtered = this.filtered();
    if (key === "escape") {
      this.onCancel();
      return;
    }
    if (key === "enter") {
      const item = filtered[this.selected];
      if (item) this.onSelect(item);
      return;
    }
    if ((key === "up" || key === "down") && filtered.length > 0) {
      const delta = key === "up" ? -1 : 1;
      this.selected =
        (this.selected + delta + filtered.length) % filtered.length;
      this.selectedId = filtered[this.selected]?.id;
      this.requestRender();
      return;
    }
    const before = this.input.getValue();
    this.input.handleInput(data);
    if (before !== this.input.getValue()) {
      this.selected = 0;
      this.selectedId = undefined;
      this.requestRender();
    }
  }

  render(width: number): string[] {
    const available = Math.max(1, width);
    this.input.focused = this.focused;
    const filtered = this.filtered();
    this.selected = Math.min(this.selected, Math.max(0, filtered.length - 1));
    const lines = [
      fit(style.secondary(this.title), available),
      ...this.input.render(available),
      "",
    ];
    if (filtered.length === 0) {
      lines.push(
        style.muted(
          fit("  No matches — edit your search or press Esc", available),
        ),
      );
      return lines;
    }

    const compact = filtered.every((item) => item.category !== undefined);
    const maxRows = Math.max(
      1,
      Math.min(
        compact ? 14 : 7,
        Math.floor(
          (this.availableRows() - 3) / (!compact && available >= 24 ? 2 : 1),
        ),
      ),
    );
    const start = Math.max(
      0,
      Math.min(
        this.selected - Math.floor(maxRows / 2),
        filtered.length - maxRows,
      ),
    );
    for (
      let index = start;
      index < Math.min(filtered.length, start + maxRows);
      index++
    ) {
      const item = filtered[index];
      if (!item) continue;
      const selected = index === this.selected;
      const marker = selected && process.env.NO_COLOR !== undefined ? "›" : " ";
      const group = fit(item.category ?? "", 7);
      const category =
        item.category && available >= 30
          ? `${" ".repeat(7 - visibleWidth(group))}${group}  `
          : "";
      const badge = fit(item.badge ?? "", Math.floor(available / 3));
      const label = fit(
        item.label,
        available - visibleWidth(category) - visibleWidth(badge) - 3,
      );
      const gap = " ".repeat(
        Math.max(1, available - visibleWidth(category + label + badge) - 2),
      );
      lines.push(
        selected
          ? style.selected(
              fit(`${marker}${category}${label}${gap}${badge} `, available),
            )
          : fit(
              ` ${style.secondary(category)}${style.bold(label)}${gap}${style.secondary(badge)} `,
              available,
            ),
      );
      if (!item.category && item.detail && available >= 24)
        lines.push(fit(`    ${style.muted(item.detail)}`, available));
    }
    return lines;
  }

  invalidate(): void {
    this.input.invalidate();
  }
}
