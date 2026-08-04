import assert from "node:assert/strict";
import test, { mock } from "node:test";
import { finalizeEvent, getPublicKey } from "nostr-tools/pure";

import { relayClient } from "@/shared/api/relayClient";
import { emojiAvatarDataUrl } from "@/features/profile/ui/ProfileAvatarEditor.utils.ts";
import {
  catalogPersonasFromPublications,
  catalogPublicationsFromEvents,
  fetchPersonaCatalogPublications,
  personaEventIsShared,
} from "./personaCatalogRelay.ts";

const ALICE_SECRET = new Uint8Array(32);
ALICE_SECRET[31] = 1;
const BOB_SECRET = new Uint8Array(32);
BOB_SECRET[31] = 2;
const ALICE = getPublicKey(ALICE_SECRET);
const BOB = getPublicKey(BOB_SECRET);

function secretForOwner(owner) {
  if (owner === ALICE) return ALICE_SECRET;
  if (owner === BOB) return BOB_SECRET;
  throw new Error(`No test secret for catalog owner ${owner}`);
}

function personaEvent({
  createdAt,
  id,
  owner = ALICE,
  sourcePersonaId = "reviewer",
  shared = true,
  avatarUrl = null,
  displayName = "Relay Reviewer",
  respondTo = null,
  systemPrompt = "Review changes.",
  sharedTag,
  contentOverride,
}) {
  return finalizeEvent(
    {
      created_at: createdAt,
      kind: 30175,
      tags: [
        ["d", sourcePersonaId],
        ["test-id", id],
        ...(shared
          ? [sharedTag ?? ["shared", "true"]]
          : sharedTag
            ? [sharedTag]
            : []),
      ],
      content:
        contentOverride ??
        JSON.stringify({
          display_name: displayName,
          system_prompt: systemPrompt,
          avatar_url: avatarUrl,
          runtime: "goose",
          model: "claude",
          provider: null,
          name_pool: ["Reviewer"],
          respond_to: respondTo,
          respond_to_allowlist: respondTo === "allowlist" ? [BOB] : undefined,
          parallelism: 4,
        }),
    },
    secretForOwner(owner),
  );
}

test("a shared kind 30175 persona from Alice is discoverable by Bob", () => {
  const publications = catalogPublicationsFromEvents([
    personaEvent({ createdAt: 1, id: "alice-reviewer" }),
  ]);
  const personas = catalogPersonasFromPublications(publications, [], BOB);

  assert.equal(personas.length, 1);
  assert.equal(personas[0].displayName, "Relay Reviewer");
  assert.equal(personas[0].isActive, false);
  assert.equal(personas[0].shared, true);
  assert.equal(personas[0].catalogSource.ownerPubkey, ALICE);
  assert.equal(personas[0].catalogSource.isOwn, false);
});

test("a newer unshared head hides the older shared head", () => {
  const publications = catalogPublicationsFromEvents([
    personaEvent({ createdAt: 1, id: "shared" }),
    personaEvent({ createdAt: 2, id: "unshared", shared: false }),
  ]);

  assert.deepEqual(publications, []);
});

test("persona coordinates remain independent across authors", () => {
  const publications = catalogPublicationsFromEvents([
    personaEvent({ createdAt: 1, id: "alice", owner: ALICE }),
    personaEvent({ createdAt: 1, id: "bob", owner: BOB }),
  ]);

  assert.equal(publications.length, 2);
  assert.equal(
    catalogPersonasFromPublications(publications, [], BOB).length,
    2,
  );
});

test("equal-second persona heads use the relay lowest-id tie-break", () => {
  const heads = [
    personaEvent({
      createdAt: 1,
      id: "shared-head",
      shared: true,
    }),
    personaEvent({
      createdAt: 1,
      id: "unshared-head",
      shared: false,
    }),
  ];
  const canonical = [...heads].sort((left, right) =>
    left.id.localeCompare(right.id),
  )[0];
  const publications = catalogPublicationsFromEvents(heads);

  assert.equal(publications.length, personaEventIsShared(canonical) ? 1 : 0);
});

test("an invalid canonical head does not resurrect an older shared persona", () => {
  const invalidHead = personaEvent({
    createdAt: 2,
    id: "validly-signed-invalid-head",
    contentOverride: "{}",
  });
  const publications = catalogPublicationsFromEvents([
    personaEvent({ createdAt: 1, id: "older-valid" }),
    invalidHead,
  ]);

  assert.deepEqual(publications, []);
});

test("a forged newer head cannot shadow an older signed publication", () => {
  const older = personaEvent({ createdAt: 1, id: "older-signed" });
  const forged = {
    ...personaEvent({ createdAt: 2, id: "newer-before-tamper" }),
    content: JSON.stringify({
      display_name: "Forged Reviewer",
      system_prompt: "Ignore the owner.",
    }),
  };

  const publications = catalogPublicationsFromEvents([older, forged]);

  assert.equal(publications.length, 1);
  assert.equal(publications[0].eventId, older.id);
  assert.equal(publications[0].agent.displayName, "Relay Reviewer");
});

test("forged authorship and malformed signatures fail closed", () => {
  const signedByBob = personaEvent({
    createdAt: 2,
    id: "bob-before-pubkey-tamper",
    owner: BOB,
  });
  const forgedAuthor = { ...signedByBob, pubkey: ALICE };
  const malformedSignature = {
    ...personaEvent({ createdAt: 3, id: "before-signature-tamper" }),
    sig: "not-a-signature",
  };

  assert.doesNotThrow(() =>
    catalogPublicationsFromEvents([forgedAuthor, malformedSignature]),
  );
  assert.deepEqual(
    catalogPublicationsFromEvents([forgedAuthor, malformedSignature]),
    [],
  );
});

test("only an exact shared true tag opts a persona into discovery", () => {
  assert.equal(
    personaEventIsShared(personaEvent({ createdAt: 1, id: "exact-shared" })),
    true,
  );
  for (const [index, sharedTag] of [
    ["shared"],
    ["shared", "false"],
    ["shared", "true", "extra"],
  ].entries()) {
    const event = personaEvent({
      createdAt: index + 2,
      id: `malformed-${index}`,
      shared: false,
      sharedTag,
    });
    assert.equal(personaEventIsShared(event), false);
    assert.deepEqual(catalogPublicationsFromEvents([event]), []);
  }
  const duplicate = personaEvent({
    createdAt: 5,
    id: "duplicate",
  });
  duplicate.tags.push(["shared", "true"]);
  assert.equal(personaEventIsShared(duplicate), false);
});

test("catalog avatars keep bounded http URLs and drop unsafe schemes", () => {
  const safe = catalogPersonasFromPublications(
    catalogPublicationsFromEvents([
      personaEvent({
        createdAt: 1,
        id: "safe-avatar",
        avatarUrl: "https://relay.example/avatar.png",
      }),
    ]),
    [],
    BOB,
  );
  assert.equal(safe[0].avatarUrl, "https://relay.example/avatar.png");

  const unsafe = catalogPersonasFromPublications(
    catalogPublicationsFromEvents([
      personaEvent({
        createdAt: 1,
        id: "unsafe-avatar",
        avatarUrl: "javascript:alert(1)",
      }),
    ]),
    [],
    BOB,
  );
  assert.equal(unsafe[0].avatarUrl, null);
});

test("catalog rejects invisible or bidirectional formatting characters", () => {
  for (const [index, character] of [
    "\u00ad",
    "\u034f",
    "\u200b",
    "\u202e",
    "\u2060",
    "\u2066",
    "\u3164",
    "\u{e007f}",
  ].entries()) {
    assert.deepEqual(
      catalogPublicationsFromEvents([
        personaEvent({
          createdAt: index + 1,
          displayName: `Review${character}er`,
          id: `unsafe-name-${index}`,
        }),
      ]),
      [],
    );
    assert.deepEqual(
      catalogPublicationsFromEvents([
        personaEvent({
          createdAt: index + 1,
          id: `unsafe-prompt-${index}`,
          systemPrompt: `Review code.${character}`,
        }),
      ]),
      [],
    );
  }
});

test("catalog keeps rendered emoji sequences in names and instructions", () => {
  for (const [index, emoji] of [
    "❤️",
    "☕️",
    "👩‍💻",
    "🧑🏽‍💻",
    "👨‍👩‍👧‍👦",
    "1️⃣",
  ].entries()) {
    const publications = catalogPublicationsFromEvents([
      personaEvent({
        createdAt: index + 1,
        displayName: `Reviewer ${emoji}`,
        id: `rendered-emoji-${index}`,
        systemPrompt: `Review changes ${emoji}`,
      }),
    ]);

    assert.equal(publications.length, 1);
    assert.equal(publications[0].agent.displayName, `Reviewer ${emoji}`);
    assert.equal(publications[0].agent.systemPrompt, `Review changes ${emoji}`);
  }
});

test("catalog rejects detached emoji formatting and tag sequences", () => {
  const taggedFlag = "🏴\u{e0067}\u{e0062}\u{e0073}\u{e0063}\u{e0074}\u{e007f}";
  for (const [index, value] of [
    "Review\ufe0fer",
    "Review\u200der",
    "Review code.\u200d",
    taggedFlag,
  ].entries()) {
    assert.deepEqual(
      catalogPublicationsFromEvents([
        personaEvent({
          createdAt: index + 1,
          displayName: value,
          id: `detached-emoji-name-${index}`,
        }),
      ]),
      [],
    );
    assert.deepEqual(
      catalogPublicationsFromEvents([
        personaEvent({
          createdAt: index + 1,
          id: `detached-emoji-prompt-${index}`,
          systemPrompt: value,
        }),
      ]),
      [],
    );
  }
});

test("catalog rejects layout controls in display names", () => {
  for (const [index, character] of ["\n", "\t"].entries()) {
    assert.deepEqual(
      catalogPublicationsFromEvents([
        personaEvent({
          createdAt: index + 1,
          displayName: `Relay${character}Reviewer`,
          id: `unsafe-layout-name-${index}`,
        }),
      ]),
      [],
    );
  }
});

test("catalog keeps visible unicode and literal markdown instructions", () => {
  const systemPrompt =
    "Review changes.\n\t||This syntax must be shown literally.||";
  const publications = catalogPublicationsFromEvents([
    personaEvent({
      createdAt: 1,
      displayName: "Relay Reviewer 🐝",
      id: "visible-unicode",
      systemPrompt,
    }),
  ]);

  assert.equal(publications[0].agent.displayName, "Relay Reviewer 🐝");
  assert.equal(publications[0].agent.systemPrompt, systemPrompt);
});

/** The avatar a catalog entry projects for `avatarUrl`, or null if dropped. */
function catalogAvatarUrl(avatarUrl) {
  const personas = catalogPersonasFromPublications(
    catalogPublicationsFromEvents([
      personaEvent({ createdAt: 1, id: "avatar-vector", avatarUrl }),
    ]),
    [],
    BOB,
  );
  return personas[0].avatarUrl;
}

// An emoji avatar is self-contained, so it is the one `data:` avatar that can
// render on another member's machine. Dropping it left shared agents looking
// avatar-less in the catalog.
test("test_percent_encoded_emoji_svg_avatar_survives_the_catalog", () => {
  const emojiAvatar = emojiAvatarDataUrl("🐝", "#FFCC00");

  assert.equal(catalogAvatarUrl(emojiAvatar), emojiAvatar);
});

test("test_base64_svg_avatar_is_rejected", () => {
  assert.equal(
    catalogAvatarUrl(`data:image/svg+xml;base64,${btoa("<svg/>")}`),
    null,
  );
});

test("test_non_svg_data_avatar_is_rejected", () => {
  assert.equal(catalogAvatarUrl("data:image/png,%89PNG"), null);
});

test("test_legacy_inline_raster_avatar_survives_the_catalog", () => {
  for (const mime of ["png", "jpeg", "gif", "webp"]) {
    const avatar = `data:image/${mime};base64,iVBORw0KGgo=`;
    assert.equal(catalogAvatarUrl(avatar), avatar);
  }
});

test("test_inline_raster_avatar_rejects_unbounded_or_malformed_payloads", () => {
  const prefix = "data:image/png;base64,";
  const payloadLength = 256 * 1_024 - prefix.length;
  const validPayloadLength = payloadLength - (payloadLength % 4);
  const withinCap = `${prefix}${"a".repeat(validPayloadLength - 2)}==`;
  assert.ok(withinCap.length <= 256 * 1_024);
  assert.equal(catalogAvatarUrl(withinCap), withinCap);
  assert.equal(
    catalogAvatarUrl(
      `${withinCap}${"a".repeat(256 * 1_024 - withinCap.length + 1)}`,
    ),
    null,
  );
  assert.equal(catalogAvatarUrl("data:image/png;base64,not base64"), null);
  assert.equal(catalogAvatarUrl("data:image/bmp;base64,aA=="), null);
});

test("test_oversized_inline_svg_avatar_is_rejected", () => {
  const withinCap = `data:image/svg+xml,${"a".repeat(8_192 - "data:image/svg+xml,".length)}`;
  assert.equal(withinCap.length, 8_192);
  assert.equal(catalogAvatarUrl(withinCap), withinCap);
  assert.equal(catalogAvatarUrl(`${withinCap}a`), null);
});

// Catalog avatars render through `<img src>` (ProfileAvatar → AvatarImage),
// where an SVG document is never scripted, so a script-bearing avatar is
// accepted and inert rather than filtered — the projection must not silently
// start sanitizing markup it does not render.
test("test_script_bearing_inline_svg_avatar_is_accepted_and_rendered_inert", () => {
  const scripted = `data:image/svg+xml,${encodeURIComponent(
    '<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>',
  )}`;

  assert.equal(catalogAvatarUrl(scripted), scripted);
});

test("foreign allowlist behavior imports as owner-only", () => {
  const personas = catalogPersonasFromPublications(
    catalogPublicationsFromEvents([
      personaEvent({
        createdAt: 1,
        id: "allowlist",
        respondTo: "allowlist",
      }),
    ]),
    [],
    BOB,
  );

  assert.equal(personas[0].respondTo, "owner-only");
  assert.deepEqual(personas[0].respondToAllowlist, []);
});

test("a pending local share does not appear before relay confirmation", () => {
  const localPersona = {
    id: "local-reviewer",
    displayName: "Local Reviewer",
    avatarUrl: null,
    systemPrompt: "Review local changes.",
    runtime: null,
    model: null,
    provider: null,
    namePool: [],
    isBuiltIn: false,
    isActive: true,
    shared: true,
    sourceTeam: null,
    envVars: {},
    respondTo: null,
    respondToAllowlist: [],
    parallelism: null,
    createdAt: "2026-07-26T00:00:00.000Z",
    updatedAt: "2026-07-26T00:00:00.000Z",
  };

  const personas = catalogPersonasFromPublications([], [localPersona], ALICE);
  assert.deepEqual(personas, []);
});

function localPersona(overrides = {}) {
  return {
    id: "local-1",
    displayName: "Relay Reviewer",
    avatarUrl: null,
    systemPrompt: "Review changes.",
    runtime: null,
    model: null,
    provider: null,
    namePool: [],
    isBuiltIn: false,
    isActive: true,
    shared: false,
    sourceTeam: null,
    catalogSource: null,
    envVars: {},
    respondTo: null,
    respondToAllowlist: [],
    parallelism: null,
    createdAt: "2026-07-26T00:00:00.000Z",
    updatedAt: "2026-07-26T00:00:00.000Z",
    ...overrides,
  };
}

// The duplicate-add bug: a copy of Alice's entry carries a fresh local UUID, so
// matching by id finds nothing and the catalog offers "Add" again. Only the
// stored catalogSource coordinate links the copy back to the publication.
test("test_added_foreign_catalog_entry_keeps_publisher_identity_and_local_selection", () => {
  const publisherAvatar = "https://relay.example/publisher.png";
  const publications = catalogPublicationsFromEvents([
    personaEvent({
      createdAt: 1,
      id: "alice-reviewer",
      avatarUrl: publisherAvatar,
    }),
  ]);
  const copy = localPersona({
    id: "a-fresh-uuid",
    displayName: "Locally Renamed Reviewer",
    avatarUrl: "https://relay.example/local-copy.png",
    systemPrompt: "Locally edited instructions.",
    catalogSource: { ownerPubkey: ALICE, personaId: "reviewer" },
  });

  const personas = catalogPersonasFromPublications(publications, [copy], BOB);

  assert.equal(personas.length, 1);
  assert.equal(
    personas[0].id,
    "a-fresh-uuid",
    "the projection must retain the existing local copy's linkage id",
  );
  assert.equal(
    personas[0].isActive,
    true,
    "an added foreign entry must read as already selected",
  );
  assert.equal(personas[0].displayName, "Relay Reviewer");
  assert.equal(personas[0].avatarUrl, publisherAvatar);
  assert.equal(personas[0].systemPrompt, "Review changes.");
});

test("test_foreign_entry_with_no_local_copy_stays_unselected", () => {
  const publications = catalogPublicationsFromEvents([
    personaEvent({ createdAt: 1, id: "alice-reviewer" }),
  ]);
  // A same-named local persona with no provenance is a different agent.
  const unrelated = localPersona({ id: "unrelated" });

  const personas = catalogPersonasFromPublications(
    publications,
    [unrelated],
    BOB,
  );

  assert.equal(personas[0].id, `catalog:${ALICE}:reviewer`);
  assert.equal(personas[0].isActive, false);
});

// Provenance is per-owner: the same d-tag under a different publisher is a
// different agent, so a copy of Alice's must not mask Bob's entry.
test("test_catalog_source_match_is_scoped_to_the_publishing_owner", () => {
  const publications = catalogPublicationsFromEvents([
    personaEvent({ createdAt: 1, id: "bob-reviewer", owner: BOB }),
  ]);
  const copyOfAlices = localPersona({
    id: "copy-of-alices",
    catalogSource: { ownerPubkey: ALICE, personaId: "reviewer" },
  });

  const personas = catalogPersonasFromPublications(
    publications,
    [copyOfAlices],
    ALICE,
  );

  assert.equal(personas[0].id, `catalog:${BOB}:reviewer`);
  assert.equal(personas[0].isActive, false);
});

test("test_own_publication_still_resolves_by_local_id", () => {
  const publications = catalogPublicationsFromEvents([
    personaEvent({ createdAt: 1, id: "alice-reviewer" }),
  ]);
  const own = localPersona({ id: "reviewer", shared: true });

  const personas = catalogPersonasFromPublications(publications, [own], ALICE);

  assert.equal(personas[0].id, "reviewer");
  assert.equal(personas[0].catalogSource.isOwn, true);
});

function pageOfEvents(count, startId, createdAt) {
  return Array.from({ length: count }, (_, index) =>
    personaEvent({
      createdAt: typeof createdAt === "function" ? createdAt(index) : createdAt,
      id: `event-${startId + index}`,
      sourcePersonaId: `persona-${startId + index}`,
    }),
  );
}

function stubPagedRelay(pages) {
  const filters = [];
  mock.method(relayClient, "fetchEvents", (filter) => {
    filters.push(filter);
    return Promise.resolve(pages[filters.length - 1] ?? []);
  });
  return filters;
}

// A single limit-capped fetch drops every entry past the relay's clamp, making
// those agents undiscoverable. The walk must keep going while pages come back
// full, and must carry an `until` cursor derived from the oldest event seen.
test("test_full_page_is_followed_by_a_cursored_request_for_older_events", async (t) => {
  t.after(() => mock.restoreAll());
  const filters = stubPagedRelay([
    pageOfEvents(500, 0, (index) => 10_000 - index),
    pageOfEvents(3, 500, 9_000),
  ]);

  const publications = await fetchPersonaCatalogPublications();

  assert.equal(filters.length, 2, "a full page must be followed by another");
  assert.equal(filters[0].until, undefined, "the first page has no cursor");
  assert.equal(
    filters[1].until,
    10_000 - 499,
    "the cursor must be the oldest created_at from the previous page",
  );
  assert.equal(
    publications.length,
    503,
    "entries past the first page must still be discoverable",
  );
});

test("test_invalid_events_cannot_control_the_catalog_cursor", async (t) => {
  t.after(() => mock.restoreAll());
  const validEvents = pageOfEvents(499, 0, (index) => 10_000 - index);
  const invalidOldest = {
    ...personaEvent({
      createdAt: 1,
      id: "invalid-oldest-cursor",
      sourcePersonaId: "invalid-oldest-cursor",
    }),
    sig: "not-a-signature",
  };
  const filters = stubPagedRelay([
    [...validEvents, invalidOldest],
    pageOfEvents(1, 500, 9_000),
  ]);

  const publications = await fetchPersonaCatalogPublications();

  assert.equal(filters.length, 2);
  assert.equal(
    filters[1].until,
    10_000 - 498,
    "the cursor must be derived only from verified events",
  );
  assert.equal(publications.length, 500);
  assert.equal(
    publications.some(
      (publication) => publication.sourcePersonaId === "invalid-oldest-cursor",
    ),
    false,
  );
});

test("test_short_first_page_does_not_issue_a_second_request", async (t) => {
  t.after(() => mock.restoreAll());
  const filters = stubPagedRelay([pageOfEvents(2, 0, 10_000)]);

  const publications = await fetchPersonaCatalogPublications();

  assert.equal(filters.length, 1);
  assert.equal(publications.length, 2);
});

// `until` is inclusive on the relay, so consecutive pages overlap on the
// boundary timestamp. Without id dedupe the repeats would be counted twice.
test("test_overlapping_pages_are_deduped_by_event_id", async (t) => {
  t.after(() => mock.restoreAll());
  const firstPage = pageOfEvents(500, 0, (index) => 10_000 - index);
  const secondPage = [
    // The boundary event repeats because `until` includes its timestamp.
    firstPage[firstPage.length - 1],
    ...pageOfEvents(2, 500, 9_000),
  ];
  stubPagedRelay([firstPage, secondPage]);

  const publications = await fetchPersonaCatalogPublications();

  assert.equal(publications.length, 502, "the repeated event must count once");
});

// The stop-on-no-progress guard: a full page whose events all share one
// created_at cannot advance the cursor, so paging must terminate instead of
// re-requesting the same page forever.
test("test_full_page_of_tied_timestamps_terminates_the_walk", async (t) => {
  t.after(() => mock.restoreAll());
  const tiedPage = pageOfEvents(500, 0, 10_000);
  const filters = stubPagedRelay([tiedPage, tiedPage, tiedPage, tiedPage]);

  const publications = await fetchPersonaCatalogPublications();

  assert.equal(
    filters.length,
    2,
    "the walk must stop once a page contributes nothing new",
  );
  assert.equal(publications.length, 500);
});
