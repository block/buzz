import {
  isReservedCommunityAgentName,
  RESERVED_COMMUNITY_AGENT_NAMES,
} from "@/features/agents/lib/reservedAgentNames";
import { normalizePubkey } from "@/shared/lib/pubkey";

/** Live #hula OpenClaw bots on buzz.huladesk.com — last-resort hard route. */
export const HULA_RESERVED_COMMUNITY_BOT_PUBKEYS: Readonly<
  Record<(typeof RESERVED_COMMUNITY_AGENT_NAMES)[number], string>
> = {
  Captain: "96d359105b220277bbd41178255689eb6cf0bc1869ac8f91e585e0d5d52a3b80",
  Mo: "d5c385179f67f8965e567c6721bd93f494bc74ef0ed67499b5842589564083ae",
  Stitch: "54d8ee67ae6bb50255b851ac53a5b4d235497e0c8ea6a43d5b05850389c531f7",
  Quasar: "1d0eb893fe0cdceb3554b383e1b71898046007b334c0151a78ad72c78bf95f28",
  Korg: "100cd48174125530e741de69c966527bf6ec945f695ee61ed9cf66eb2cac94fb",
};

function normalizeLabel(name: string): string {
  return name.trim().toLocaleLowerCase();
}

export type ReservedBotRouteSource = {
  name: string;
  pubkey: string;
};

/**
 * Map reserved community names → pubkey.
 * Precedence: installed community-bots catalog, then channel bot members,
 * then the hard-coded Hula #hula pubkeys.
 */
export function reservedCommunityBotRoutes({
  catalogBots = [],
  channelBots = [],
  useHulaFallback = true,
}: {
  catalogBots?: ReadonlyArray<ReservedBotRouteSource>;
  channelBots?: ReadonlyArray<ReservedBotRouteSource>;
  useHulaFallback?: boolean;
} = {}): Map<string, string> {
  const routes = new Map<string, string>();

  const take = (entries: ReadonlyArray<ReservedBotRouteSource>) => {
    for (const entry of entries) {
      if (!isReservedCommunityAgentName(entry.name)) continue;
      const key = normalizeLabel(entry.name);
      if (routes.has(key)) continue;
      const pubkey = normalizePubkey(entry.pubkey);
      if (!/^[0-9a-f]{64}$/.test(pubkey)) continue;
      routes.set(key, pubkey);
    }
  };

  take(catalogBots);
  take(channelBots);

  if (useHulaFallback) {
    for (const name of RESERVED_COMMUNITY_AGENT_NAMES) {
      const key = normalizeLabel(name);
      if (routes.has(key)) continue;
      routes.set(key, normalizePubkey(HULA_RESERVED_COMMUNITY_BOT_PUBKEYS[name]));
    }
  }

  return routes;
}

export function reservedRoutePubkey(
  displayName: string,
  routes: ReadonlyMap<string, string>,
): string | null {
  if (!isReservedCommunityAgentName(displayName)) return null;
  return routes.get(normalizeLabel(displayName)) ?? null;
}

/**
 * Drop non-community competitors that claim a reserved exact label, and keep
 * the routed community bot when present.
 */
export function applyReservedCommunityMentionRouting<
  T extends { displayName?: string | null; pubkey?: string | null },
>(candidates: readonly T[], routes: ReadonlyMap<string, string>): T[] {
  if (routes.size === 0) return [...candidates];

  return candidates.filter((candidate) => {
    const label = candidate.displayName?.trim() ?? "";
    if (!isReservedCommunityAgentName(label)) return true;
    const routed = routes.get(normalizeLabel(label));
    if (!routed) return true;
    if (!candidate.pubkey) return false;
    return normalizePubkey(candidate.pubkey) === routed;
  });
}

/** Force selected mention bindings for reserved labels onto community bots. */
export function rewriteSelectedMentionsForReservedRoutes(
  selectedMentions: ReadonlyMap<string, string>,
  routes: ReadonlyMap<string, string>,
): Map<string, string> {
  const next = new Map<string, string>();
  for (const [label, pubkey] of selectedMentions) {
    const routed = reservedRoutePubkey(label, routes);
    next.set(label, routed ?? pubkey);
  }
  return next;
}
