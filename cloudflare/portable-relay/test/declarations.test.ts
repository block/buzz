import { finalizeEvent, generateSecretKey, getPublicKey } from "nostr-tools";
import { describe, expect, it } from "vitest";
import {
  evaluateDeclarations,
  KIND_SYNC_DECLARATION,
  MalformedDeclarationError,
} from "../src/declarations";

const ownerSecret = generateSecretKey();
const owner = getPublicKey(ownerSecret);
const strangerSecret = generateSecretKey();
const peerKey = getPublicKey(generateSecretKey());
const NODE = "cf-rendezvous";

function declaration(
  d: string,
  content: Record<string, unknown>,
  options: { p?: string[]; secret?: Uint8Array; node?: string } = {},
) {
  return finalizeEvent(
    {
      kind: KIND_SYNC_DECLARATION,
      created_at: 1_785_100_000,
      tags: [
        ["d", d],
        ["n", options.node ?? NODE],
        ...(options.p ?? []).map((key) => ["p", key]),
      ],
      content: JSON.stringify(content),
    },
    options.secret ?? ownerSecret,
  );
}

describe("evaluateDeclarations", () => {
  it("reports every domain unclaimed with no heads", () => {
    const config = evaluateDeclarations([], owner, NODE);
    expect(config).toEqual({ peers: null, readers: null, streams: null });
  });

  it("evaluates active admit, read, and export heads into their domains", () => {
    const config = evaluateDeclarations(
      [
        declaration(
          "admit/node-c/work",
          { status: "active", principal: "did:buzz:node-c" },
          { p: [peerKey] },
        ),
        declaration(
          "read/ted-laptop/sessions-buzz",
          { status: "active", principal: "did:buzz:node-b" },
          { p: [peerKey] },
        ),
        declaration("export/ted-laptop/sessions-buzz", {
          status: "active",
          selection: { filter: [{ kinds: [1] }] },
        }),
      ],
      owner,
      NODE,
    );
    expect(config.peers).toEqual({
      "node-c/work": {
        principal: "did:buzz:node-c",
        verification_keys: [peerKey],
      },
    });
    expect(config.readers).toEqual({
      "ted-laptop/sessions-buzz": {
        principal: "did:buzz:node-b",
        verification_keys: [peerKey],
      },
    });
    expect(config.streams).toEqual({
      "ted-laptop/sessions-buzz": {
        mode: "filter",
        filters: [{ kinds: [1] }],
      },
    });
  });

  it("claims a domain with a revoked head without conferring trust", () => {
    const config = evaluateDeclarations(
      [
        declaration(
          "admit/node-b/demo",
          { status: "revoked", principal: "did:buzz:node-b" },
          { p: [peerKey] },
        ),
      ],
      owner,
      NODE,
    );
    expect(config.peers).toEqual({});
    expect(config.readers).toBeNull();
    expect(config.streams).toBeNull();
  });

  it("ignores heads n-tagged for a different node", () => {
    // A replicated journal carries the owner's declarations for every node;
    // the laptop's sink trust must not claim the rendezvous's peers domain.
    const config = evaluateDeclarations(
      [
        declaration(
          "admit/node-b/demo",
          { status: "active", principal: "did:buzz:node-b" },
          { p: [peerKey], node: "ted-laptop" },
        ),
      ],
      owner,
      NODE,
    );
    expect(config.peers).toBeNull();
  });

  it("ignores heads not signed by the owner", () => {
    const config = evaluateDeclarations(
      [
        declaration(
          "admit/intruder/stream",
          { status: "active", principal: "did:buzz:intruder" },
          { p: [peerKey], secret: strangerSecret },
        ),
      ],
      owner,
      NODE,
    );
    expect(config.peers).toBeNull();
  });

  it("throws on an active admit head without verification keys", () => {
    expect(() =>
      evaluateDeclarations(
        [
          declaration("admit/node-c/work", {
            status: "active",
            principal: "did:buzz:node-c",
          }),
        ],
        owner,
      NODE,
      ),
    ).toThrow(MalformedDeclarationError);
  });

  it("throws on an active export head with a non-normative selection", () => {
    expect(() =>
      evaluateDeclarations(
        [
          declaration("export/bad", {
            status: "active",
            selection: { filter: null },
          }),
        ],
        owner,
      NODE,
      ),
    ).toThrow(MalformedDeclarationError);
  });

  it("evaluates mirror and from_source selections", () => {
    const config = evaluateDeclarations(
      [
        declaration("export/whole", {
          status: "active",
          selection: { mirror: true },
        }),
        declaration("export/provenance", {
          status: "active",
          selection: { from_source: "node-c/work" },
        }),
      ],
      owner,
      NODE,
    );
    expect(config.streams).toEqual({
      whole: { mode: "mirror" },
      provenance: { mode: "from_source", source: "node-c/work" },
    });
  });
});
