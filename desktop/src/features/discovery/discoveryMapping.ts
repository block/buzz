/**
 * Turns one provider place into the fields of a prospect record. Pure: the
 * limits below mirror `validate_prospect_action` in the relay
 * (crates/buzz-relay/src/handlers/business_records.rs), which counts bytes.
 */

import type { ProspectChanges } from "./businessRecordMutations";
import { stableRecordId } from "./businessRecordContract";
import type { DiscoveryTagMatch } from "./discoveryTags";
import type { DiscoveryPlace } from "./discoverySearch";

const MAX_NAME_BYTES = 240;
const MAX_TAXONOMY_BYTES = 200;
const MAX_WEBSITE_BYTES = 2_048;
const MAX_LOCATION_BYTES = 200;
const MAX_EMAIL_BYTES = 254;
const MAX_PHONE_BYTES = 50;
const MAX_EXTERNAL_ID_BYTES = 128;
const MAX_EVIDENCE_TITLE_BYTES = 200;
const MAX_EVIDENCE_EXCERPT_BYTES = 2_000;

export const EVIDENCE_TITLE = "Google Maps listing";

const encoder = new TextEncoder();

/** Trims, then cuts to at most `maxBytes` UTF-8 bytes without splitting a character. */
export function clampBytes(value: string, maxBytes: number): string {
  const trimmed = value.trim();
  if (encoder.encode(trimmed).length <= maxBytes) return trimmed;
  let result = "";
  let used = 0;
  for (const character of trimmed) {
    const size = encoder.encode(character).length;
    if (used + size > maxBytes) break;
    result += character;
    used += size;
  }
  return result.trim();
}

function present(value: string | null | undefined): string | null {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  return trimmed ? trimmed : null;
}

/** An https URL for `value`: http is upgraded, anything else is dropped. */
export function httpsUrl(
  value: string | null | undefined,
  maxBytes = MAX_WEBSITE_BYTES,
): string | null {
  const raw = present(value);
  if (!raw) return null;
  let parsed: URL;
  try {
    parsed = new URL(raw);
  } catch {
    return null;
  }
  if (parsed.protocol === "http:") parsed.protocol = "https:";
  if (parsed.protocol !== "https:" || !parsed.hostname) return null;
  const href = parsed.toString();
  return encoder.encode(href).length <= maxBytes ? href : null;
}

/** How business names are compared when deciding if a lead already exists. */
export function leadNameKey(name: string): string {
  return name.trim().replace(/\s+/g, " ").toLocaleLowerCase();
}

/** A stable key for a place: its provider id, else its name and address. */
export function placeKey(place: DiscoveryPlace): string {
  const id = present(place.placeId);
  if (id) return id;
  return `${place.name.trim().toLocaleLowerCase()}|${(place.address ?? "")
    .trim()
    .toLocaleLowerCase()}`;
}

/**
 * The prospect record id for a place. Deterministic, so finding the same
 * business again (in this or a later campaign) maps to the record it already
 * has instead of creating a second one.
 */
export function placeProspectId(place: DiscoveryPlace): Promise<string> {
  return stableRecordId("discovery-place", placeKey(place));
}

/**
 * The SalesTeams quality score for a Google Maps result: 30 for having a name,
 * plus 20 phone, 15 address, 20 website, 15 rating. At most 100.
 */
export function fitScore(place: DiscoveryPlace): number {
  let score = 30;
  if (present(place.phone)) score += 20;
  if (present(place.address)) score += 15;
  if (present(place.website)) score += 20;
  if (place.rating !== null) score += 15;
  return Math.min(100, score);
}

/** "City, State" when the place has a city, otherwise its full address. */
export function locationSummary(place: DiscoveryPlace): string | null {
  const city = present(place.city);
  const state = present(place.state);
  const summary = city
    ? state && state.toLocaleLowerCase() !== city.toLocaleLowerCase()
      ? `${city}, ${state}`
      : city
    : present(place.address);
  return summary ? clampBytes(summary, MAX_LOCATION_BYTES) || null : null;
}

function evidenceExcerpt(place: DiscoveryPlace): string {
  const parts: string[] = [];
  const category = present(place.category) ?? present(place.subtypes);
  if (category) parts.push(category);
  if (place.rating !== null) {
    const reviews =
      place.reviews !== null && place.reviews > 0
        ? ` from ${Math.round(place.reviews)} ${Math.round(place.reviews) === 1 ? "review" : "reviews"}`
        : "";
    parts.push(`Rated ${place.rating}${reviews}`);
  }
  const address = present(place.address);
  if (address) parts.push(address);
  return clampBytes(parts.join(" · "), MAX_EVIDENCE_EXCERPT_BYTES);
}

export type MappingContext = {
  campaignId: string;
  industry: string;
  vertical: string;
  /** Exact tag evidence matched from the provider result. */
  discoveryTags?: DiscoveryTagMatch[];
  /** Unix seconds the place was seen. */
  nowSeconds: number;
};

/** The party external ids that tie a record to its source place and campaign. */
export function externalIdsFor(
  place: DiscoveryPlace,
  campaignId: string,
): string[] {
  const ids: string[] = [];
  const placeId = present(place.placeId);
  if (placeId) {
    const id = `outscraper:${placeId}`;
    if (encoder.encode(id).length <= MAX_EXTERNAL_ID_BYTES) ids.push(id);
  }
  const campaign = `campaign:${campaignId}`;
  if (encoder.encode(campaign).length <= MAX_EXTERNAL_ID_BYTES) {
    ids.push(campaign);
  }
  return ids;
}

/**
 * The prospect fields to publish for a place, or null when the place cannot
 * become a valid record (no name, or no usable industry or vertical).
 */
export function mapPlaceToProspect(
  place: DiscoveryPlace,
  context: MappingContext,
  prospectId: string,
): ProspectChanges | null {
  const displayName = clampBytes(place.name, MAX_NAME_BYTES);
  const industry = clampBytes(context.industry, MAX_TAXONOMY_BYTES);
  const vertical = clampBytes(context.vertical, MAX_TAXONOMY_BYTES);
  if (!displayName || !industry || !vertical) return null;

  const website = httpsUrl(place.website);
  const evidenceUrl = httpsUrl(place.mapsUrl) ?? website;
  const email = present(place.email);
  const phone = present(place.phone);

  return {
    prospectId,
    displayName,
    partyType: "organization",
    externalIds: externalIdsFor(place, context.campaignId),
    industry,
    vertical,
    fitScore: fitScore(place),
    origin: "discovery",
    discoveryTags: context.discoveryTags ?? [],
    potentialMonthlyValueMinor: null,
    website,
    contactName: null,
    location: locationSummary(place),
    email:
      email?.includes("@") && encoder.encode(email).length <= MAX_EMAIL_BYTES
        ? email
        : null,
    phone:
      phone && encoder.encode(phone).length <= MAX_PHONE_BYTES ? phone : null,
    evidence: evidenceUrl
      ? [
          {
            title: clampBytes(EVIDENCE_TITLE, MAX_EVIDENCE_TITLE_BYTES),
            url: evidenceUrl,
            excerpt: evidenceExcerpt(place),
            observedAt: context.nowSeconds,
          },
        ]
      : [],
    lastVerifiedAt: context.nowSeconds,
    qualification: "unreviewed",
    saved: false,
    stage: "qualified",
    lostReason: null,
  };
}
