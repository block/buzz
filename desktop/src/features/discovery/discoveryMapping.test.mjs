import assert from "node:assert/strict";
import test from "node:test";

import {
  clampBytes,
  externalIdsFor,
  fitScore,
  httpsUrl,
  leadNameKey,
  locationSummary,
  mapPlaceToProspect,
  placeKey,
  placeProspectId,
} from "./discoveryMapping.ts";

function place(overrides = {}) {
  return {
    placeId: "ChIJabc",
    name: "The Olive House",
    category: "Home goods store",
    subtypes: null,
    address: "12 Kloof Street, Parkhurst, Johannesburg",
    city: "Johannesburg",
    state: "Gauteng",
    country: "South Africa",
    phone: "+27 11 555 0100",
    website: "https://olive.example",
    email: "hello@olive.example",
    rating: 4.6,
    reviews: 120,
    latitude: null,
    longitude: null,
    mapsUrl: "https://maps.google.com/?cid=1",
    description: null,
    businessStatus: null,
    ...overrides,
  };
}

const context = {
  campaignId: "camp-1",
  industry: "Home & Living",
  vertical: "Home Decor & Gift Shops",
  nowSeconds: 1_700_000_000,
};

test("fit score follows the SalesTeams quality formula and caps at 100", () => {
  assert.equal(fitScore(place()), 100);
  assert.equal(
    fitScore(
      place({ phone: null, address: null, website: null, rating: null }),
    ),
    30,
  );
  assert.equal(fitScore(place({ website: null, rating: null })), 65);
  assert.equal(fitScore(place({ phone: "  ", address: "" })), 65);
});

test("prospect ids are deterministic per place and differ between places", async () => {
  const first = await placeProspectId(place());
  assert.equal(first, await placeProspectId(place()));
  assert.notEqual(
    first,
    await placeProspectId(place({ placeId: "ChIJother" })),
  );
  assert.match(
    first,
    /^[0-9a-f]{8}-[0-9a-f]{4}-5[0-9a-f]{3}-[89ab][0-9a-f]{3}-/,
  );
});

test("a place without an id is keyed by name and address, ignoring case", async () => {
  const a = place({
    placeId: null,
    name: "Form & Field",
    address: "1 Main Rd",
  });
  const b = place({
    placeId: null,
    name: "form & field",
    address: "1 MAIN RD",
  });
  assert.equal(placeKey(a), placeKey(b));
  assert.equal(await placeProspectId(a), await placeProspectId(b));
});

test("http sites are upgraded, other schemes and junk are dropped", () => {
  assert.equal(
    httpsUrl("http://olive.example/path"),
    "https://olive.example/path",
  );
  assert.equal(httpsUrl("https://olive.example"), "https://olive.example/");
  assert.equal(httpsUrl("ftp://olive.example"), null);
  assert.equal(httpsUrl("not a url"), null);
  assert.equal(httpsUrl("   "), null);
  assert.equal(httpsUrl(null), null);
  assert.equal(httpsUrl(`https://olive.example/${"a".repeat(3000)}`), null);
});

test("clampBytes counts UTF-8 bytes and never splits a character", () => {
  assert.equal(clampBytes("  hello  ", 10), "hello");
  assert.equal(clampBytes("abcdef", 3), "abc");
  const clamped = clampBytes("é".repeat(10), 5);
  assert.equal(clamped, "éé");
  assert.ok(new TextEncoder().encode(clamped).length <= 5);
});

test("location prefers city and state, then the address", () => {
  assert.equal(locationSummary(place()), "Johannesburg, Gauteng");
  assert.equal(
    locationSummary(place({ state: "Johannesburg" })),
    "Johannesburg",
  );
  assert.equal(
    locationSummary(place({ city: null })),
    "12 Kloof Street, Parkhurst, Johannesburg",
  );
  assert.equal(locationSummary(place({ city: null, address: null })), null);
});

test("external ids tie a record to its place and campaign within 128 bytes", () => {
  assert.deepEqual(externalIdsFor(place(), "camp-1"), [
    "outscraper:ChIJabc",
    "campaign:camp-1",
  ]);
  assert.deepEqual(externalIdsFor(place({ placeId: null }), "camp-1"), [
    "campaign:camp-1",
  ]);
  const long = externalIdsFor(place({ placeId: "x".repeat(200) }), "camp-1");
  assert.deepEqual(long, ["campaign:camp-1"]);
});

test("a complete place maps to a valid unreviewed prospect", () => {
  const changes = mapPlaceToProspect(place(), context, "id-1");
  assert.deepEqual(changes, {
    prospectId: "id-1",
    displayName: "The Olive House",
    partyType: "organization",
    externalIds: ["outscraper:ChIJabc", "campaign:camp-1"],
    industry: "Home & Living",
    vertical: "Home Decor & Gift Shops",
    fitScore: 100,
    origin: "discovery",
    discoveryTags: [],
    potentialMonthlyValueMinor: null,
    website: "https://olive.example/",
    contactName: null,
    location: "Johannesburg, Gauteng",
    email: "hello@olive.example",
    phone: "+27 11 555 0100",
    evidence: [
      {
        title: "Google Maps listing",
        url: "https://maps.google.com/?cid=1",
        excerpt:
          "Home goods store · Rated 4.6 from 120 reviews · 12 Kloof Street, Parkhurst, Johannesburg",
        observedAt: 1_700_000_000,
      },
    ],
    lastVerifiedAt: 1_700_000_000,
    qualification: "unreviewed",
    saved: false,
    stage: "qualified",
    lostReason: null,
  });
});

test("a sparse place still maps: no website, no evidence link, bad email dropped", () => {
  const changes = mapPlaceToProspect(
    place({
      website: null,
      mapsUrl: null,
      email: "not-an-email",
      phone: "9".repeat(80),
      rating: null,
      reviews: null,
    }),
    context,
    "id-2",
  );
  assert.equal(changes.website, null);
  assert.deepEqual(changes.evidence, []);
  assert.equal(changes.email, null);
  assert.equal(changes.phone, null);
  assert.equal(changes.fitScore, 65);
});

test("evidence falls back to the website when there is no maps link", () => {
  const changes = mapPlaceToProspect(
    place({
      mapsUrl: "http://maps.example/x",
      website: "http://olive.example",
    }),
    context,
    "id-3",
  );
  // An http maps link is upgraded, so it is still preferred.
  assert.equal(changes.evidence[0].url, "https://maps.example/x");
  const noMaps = mapPlaceToProspect(
    place({ mapsUrl: null, website: "http://olive.example" }),
    context,
    "id-3",
  );
  assert.equal(noMaps.evidence[0].url, "https://olive.example/");
});

test("values are trimmed and clamped to the relay's limits", () => {
  const changes = mapPlaceToProspect(
    place({
      name: `  ${"N".repeat(400)}  `,
      address: "A".repeat(500),
      city: null,
    }),
    { ...context, industry: "I".repeat(300), vertical: "V".repeat(300) },
    "id-4",
  );
  assert.equal(changes.displayName.length, 240);
  assert.equal(changes.industry.length, 200);
  assert.equal(changes.vertical.length, 200);
  assert.ok(changes.location.length <= 200);
  assert.ok(
    new TextEncoder().encode(changes.evidence[0].excerpt).length <= 2000,
  );
});

test("a place with no usable name or no taxonomy cannot become a record", () => {
  assert.equal(mapPlaceToProspect(place({ name: "   " }), context, "id"), null);
  assert.equal(
    mapPlaceToProspect(place(), { ...context, vertical: "  " }, "id"),
    null,
  );
});

test("lead names compare ignoring case and spacing", () => {
  assert.equal(leadNameKey("  The  Olive   House "), "the olive house");
  assert.equal(leadNameKey("FORM & FIELD"), leadNameKey("form & field"));
});
