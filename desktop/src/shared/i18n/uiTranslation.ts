import {
  createElement,
  Fragment,
  type ReactNode,
  useSyncExternalStore,
} from "react";
import { i18n } from "@/i18n";
import uiKeys from "@/locales/ui-keys.json";

const keys: Readonly<Record<string, string>> = uiKeys;

/** Translate an explicitly authored UI message. Interpolated user values stay literal. */
export function uiT(
  source: string,
  values: Readonly<
    Record<string, string | number | boolean | null | undefined>
  > = {},
): string {
  const normalized = source.replace(/\s+/g, " ").trim();
  const key = keys[normalized];
  if (!key)
    return source.replace(/\{(\d+)\}/g, (slot, index) =>
      Object.hasOwn(values, `p${index}`)
        ? String(values[`p${index}`] ?? "")
        : slot,
    );
  const prefix = source.match(/^\s*/)?.[0] ?? "";
  const suffix = source.match(/\s*$/)?.[0] ?? "";
  const fallback = normalized.replace(/\{(\d+)\}/g, (_, n) => `{{p${n}}}`);
  // Substitute only slots in the authored template. Values may themselves
  // contain i18next syntax and must never be parsed a second time.
  const template =
    i18n.getResource(i18n.resolvedLanguage ?? "en", "ui", key) ||
    i18n.getResource("en", "ui", key) ||
    fallback;
  const translated = String(template).replace(
    /\{\{(p\d+)\}\}/g,
    (slot, name) =>
      Object.hasOwn(values, name) ? String(values[name] ?? "") : slot,
  );
  return `${prefix}${translated}${suffix}`;
}

/** Reorder inline React content without coercing links or user values into HTML. */
export function uiMessage(
  source: string,
  values: Readonly<Record<string, ReactNode>>,
): ReactNode {
  const normalized = source.replace(/\s+/g, " ").trim();
  const key = keys[normalized];
  const template =
    (key &&
      (i18n.getResource(i18n.resolvedLanguage ?? "en", "ui", key) ||
        i18n.getResource("en", "ui", key))) ||
    normalized.replace(/\{(\d+)\}/g, (_, n) => `{{p${n}}}`);
  const prefix = source.match(/^\s*/)?.[0] ?? "";
  const suffix = source.match(/\s*$/)?.[0] ?? "";
  return `${prefix}${String(template)}${suffix}`
    .split(/(\{\{p\d+\}\})/g)
    .map((part, index) => {
      const parameter = /^\{\{(p\d+)\}\}$/.exec(part)?.[1];
      return createElement(
        Fragment,
        { key: index },
        parameter ? values[parameter] : part,
      );
    });
}

function subscribe(callback: () => void): () => void {
  i18n.on("languageChanged", callback);
  return () => i18n.off("languageChanged", callback);
}

function snapshot(): string {
  return i18n.resolvedLanguage ?? "en";
}

/** Keep component labels and memoized view models in sync without remounting the app. */
export function useUiLanguage(): string {
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}
