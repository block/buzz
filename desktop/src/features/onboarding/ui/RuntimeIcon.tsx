import * as React from "react";
import { TerminalSquare } from "lucide-react";

import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { BuzzMark } from "@/shared/ui/buzz-logo/BuzzMark";
import claudeLogoUrl from "../assets/harness-logos/claude.png?inline";
import { RUNTIME_MARKS } from "./HarnessMarks";

// Bundled logos for compiled-in runtimes (inline base64, no network fetch).
// Monochrome marks live in RUNTIME_MARKS instead — inline SVGs that follow
// `currentColor`, so they adapt to dark/light without bitmap filters.
const RUNTIME_LOGOS: Record<string, string> = {
  claude: claudeLogoUrl,
};

// Public-path logos for bundled presets. Served from /harness-logos/ at runtime.
// Keys match the preset `id` values emitted by the backend PRESET_HARNESSES.
export const PRESET_LOGOS: Record<string, string> = {
  omp: "/harness-logos/omp.svg",
  grok: "/harness-logos/grok.svg",
  opencode: "/harness-logos/opencode.svg",
  kimi: "/harness-logos/kimi.png",
  amp: "/harness-logos/amp.png",
  hermes: "/harness-logos/hermes.png",
  openclaw: "/harness-logos/openclaw.svg",
};

function isBuzzRuntime(runtime: AcpRuntimeCatalogEntry): boolean {
  return runtime.id.trim().toLowerCase() === "buzz-agent";
}

export function getRuntimeDisplayLabel(
  runtime: AcpRuntimeCatalogEntry,
): string {
  return isBuzzRuntime(runtime) ? "Buzz" : runtime.label;
}

// Harness ids are catalog data — a remote host names its own entries — so every
// lookup below is own-property only. A bare index would resolve `constructor`
// or `__proto__` to an inherited Object member: `RUNTIME_MARKS.constructor` is
// truthy, and rendering it as `<Mark />` throws.
function ownLookup<T>(table: Record<string, T>, id: string): T | undefined {
  return Object.hasOwn(table, id) ? table[id] : undefined;
}

/** The inline mark for a harness id, if it ships one. */
function harnessMark(id: string) {
  return ownLookup(RUNTIME_MARKS, id);
}

/** The bitmap logo url for a harness id, if it ships one. */
function harnessLogoUrl(id: string): string | null {
  return ownLookup(RUNTIME_LOGOS, id) ?? ownLookup(PRESET_LOGOS, id) ?? null;
}

/** Whether `id` names artwork of any kind — an inline mark or a bitmap logo. */
function hasHarnessArtwork(id: string): boolean {
  return Boolean(harnessMark(id)) || harnessLogoUrl(id) !== null;
}

/**
 * The id whose artwork a harness id should render.
 *
 * A remote catalog advertises one entry per identity on the host — `hermes-matt`
 * beside `hermes` — and an exact-id lookup renders every one of them as the
 * generic TerminalSquare next to the plain entry's real mark. So a full id that
 * maps nothing falls back to its base: the text before the FIRST hyphen, and
 * only when that base is itself a mapped id, so `buzz-agent` (base `buzz`,
 * unmapped) is untouched and no id can be shortened into artwork it did not
 * earn.
 *
 * Marks and logos are consulted together on purpose. They are two spellings of
 * the same thing — Goose and Cursor ship inline SVG marks, Hermes and Grok ship
 * bitmaps — so resolving against only one of them would give `hermes-matt` its
 * base's artwork while leaving `goose-nightly` on the terminal glyph.
 *
 * The resolved id is what the caller keys everything off, because the per-logo
 * backdrop classes below belong to the artwork, not to the entry: a variant
 * that borrows `omp`'s white-on-black mark needs `omp`'s dark plate with it.
 *
 * Deliberately generic: nothing here knows what a Hermes profile is. Any
 * `<known>-<variant>` id gets the known harness's artwork.
 */
function resolveHarnessArtworkId(harnessId: string): string {
  const id = harnessId.trim().toLowerCase();
  if (hasHarnessArtwork(id)) return id;
  const separator = id.indexOf("-");
  if (separator <= 0) return id;
  const base = id.slice(0, separator);
  return hasHarnessArtwork(base) ? base : id;
}

/**
 * The bundled logo url for a harness id, or `null`.
 *
 * `null` covers both "no artwork at all" and "artwork is an inline mark, which
 * has no url" — callers that need a url (pinned-harness chips) fall back to
 * their own glyph either way.
 */
export function getHarnessLogoUrl(harnessId: string): string | null {
  return harnessLogoUrl(resolveHarnessArtworkId(harnessId));
}

export function RuntimeIcon({
  className = "h-8 w-8",
  runtime,
}: {
  className?: string;
  runtime: AcpRuntimeCatalogEntry;
}) {
  const [imageFailed, setImageFailed] = React.useState(false);
  // Only use bundled artwork — never render user-supplied avatar URLs for
  // custom/preset entries (tracking pixel / spoofing vector, security line).
  //
  // The id the ARTWORK belongs to, so a variant entry (`hermes-matt`,
  // `goose-nightly`) gets its base's mark or logo — and its backdrop with it.
  const id = resolveHarnessArtworkId(runtime.id);
  const Mark = harnessMark(id);
  const imageUrl = harnessLogoUrl(id);

  if (isBuzzRuntime(runtime)) {
    // The mark's wide viewBox letterboxes inside a square box, so honoring
    // the caller's size keeps it optically in line with the square logos.
    return <BuzzMark className={cn(className, "text-foreground")} />;
  }

  if (Mark) {
    return <Mark className={cn(className, "p-0.5 text-foreground")} />;
  }

  if (imageUrl && !imageFailed) {
    return (
      <img
        alt=""
        className={cn(
          "rounded-md object-contain",
          className,
          id === "omp" && "bg-[#0d0d0d] p-1",
          id === "grok" && "bg-white p-1",
        )}
        onError={() => setImageFailed(true)}
        src={imageUrl}
      />
    );
  }

  return (
    <TerminalSquare
      className={cn(className, "text-foreground")}
      strokeWidth={1.25}
    />
  );
}
