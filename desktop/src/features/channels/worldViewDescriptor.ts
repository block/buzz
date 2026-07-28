export type ParsedPublicWorldViewReference = {
  reference:
    | {
        kind: "hosted-world-view-export";
        origin: string;
        shareToken: string;
      }
    | {
        kind: "hosted-world-live-view-share";
        origin: string;
        shareToken: string;
      };
  selection: {
    realmQualifiedName: string;
    viewQualifiedName: string;
  } | null;
};

export type ParsePublicWorldViewReferenceResult =
  | { ok: true; value: ParsedPublicWorldViewReference }
  | { ok: false; error: string };

const DEFAULT_SHIVAI_ORIGIN = "https://manifest.shivai.space";
const DESCRIPTOR_HEADER = "Shivai view reference";
const SOURCE_PREFIX = "Source: ";
const REALM_PREFIX = "Realm: ";
const VIEW_PREFIX = "View qualified: ";
const VIEW_EXPORT_PATH_PREFIX = "/world/exports/";
const LIVE_VIEW_SHARE_PATH_PREFIX = "/world/live/";

/** Parse one public Shivai view link or copied view reference. */
export function parsePublicWorldViewReference(
  input: string,
): ParsePublicWorldViewReferenceResult {
  const value = input.trim();
  if (!value) {
    return { ok: false, error: "Paste a public Shivai view link." };
  }

  const link = parseHostedPublicViewLink(value);
  if (link) {
    return { ok: true, value: { reference: link, selection: null } };
  }

  return parseCopiedViewReference(value);
}

function parseHostedPublicViewLink(
  value: string,
): ParsedPublicWorldViewReference["reference"] | null {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    return null;
  }
  const isLoopbackHttp =
    url.protocol === "http:" &&
    (url.hostname === "localhost" ||
      url.hostname === "127.0.0.1" ||
      url.hostname === "[::1]");
  if (url.protocol !== "https:" && !isLoopbackHttp) return null;
  if (url.search || url.hash) return null;
  const source = url.pathname.startsWith(LIVE_VIEW_SHARE_PATH_PREFIX)
    ? {
        kind: "hosted-world-live-view-share" as const,
        pathPrefix: LIVE_VIEW_SHARE_PATH_PREFIX,
      }
    : url.pathname.startsWith(VIEW_EXPORT_PATH_PREFIX)
      ? {
          kind: "hosted-world-view-export" as const,
          pathPrefix: VIEW_EXPORT_PATH_PREFIX,
        }
      : null;
  if (!source) return null;

  const encodedToken = url.pathname.slice(source.pathPrefix.length);
  if (!encodedToken || encodedToken.includes("/")) return null;
  let shareToken: string;
  try {
    shareToken = decodeURIComponent(encodedToken).trim();
  } catch {
    return null;
  }
  if (!shareToken) return null;

  return {
    kind: source.kind,
    origin: url.origin,
    shareToken,
  };
}

function parseCopiedViewReference(
  descriptor: string,
): ParsePublicWorldViewReferenceResult {
  const lines = descriptor
    .replaceAll("\r\n", "\n")
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
  if (lines.length !== 4 || lines[0] !== DESCRIPTOR_HEADER) {
    return {
      ok: false,
      error:
        "Paste a public Shivai view link or complete copied view reference.",
    };
  }

  const source = lines[1]?.startsWith(SOURCE_PREFIX)
    ? lines[1].slice(SOURCE_PREFIX.length)
    : null;
  if (!source) {
    return { ok: false, error: "The Shivai reference has no source." };
  }
  if (source.startsWith("local world ")) {
    return {
      ok: false,
      error: "Local paths cannot be published as channel or thread bindings.",
    };
  }
  if (source.startsWith("hosted edit share ")) {
    return {
      ok: false,
      error: "Edit-share capabilities cannot be published as bindings.",
    };
  }
  if (source.startsWith("hosted world ")) {
    return {
      ok: false,
      error: "Create a read-only hosted view export before binding this view.",
    };
  }

  const quotedShareToken = source.startsWith("hosted view export ")
    ? source.slice("hosted view export ".length)
    : null;
  if (!quotedShareToken) {
    return { ok: false, error: "This Shivai source is not shareable." };
  }
  let shareToken: unknown;
  try {
    shareToken = JSON.parse(quotedShareToken);
  } catch {
    return { ok: false, error: "The hosted export reference is malformed." };
  }
  if (typeof shareToken !== "string" || shareToken.trim().length === 0) {
    return { ok: false, error: "The hosted export reference is malformed." };
  }

  const realmQualifiedName = lines[2]?.startsWith(REALM_PREFIX)
    ? lines[2].slice(REALM_PREFIX.length).trim()
    : "";
  const viewQualifiedName = lines[3]?.startsWith(VIEW_PREFIX)
    ? lines[3].slice(VIEW_PREFIX.length).trim()
    : "";
  if (!realmQualifiedName || !viewQualifiedName) {
    return {
      ok: false,
      error: "The Shivai reference must name both a realm and a view.",
    };
  }

  return {
    ok: true,
    value: {
      reference: {
        kind: "hosted-world-view-export",
        origin: DEFAULT_SHIVAI_ORIGIN,
        shareToken,
      },
      selection: { realmQualifiedName, viewQualifiedName },
    },
  };
}
