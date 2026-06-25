/**
 * isomorphic-git wrapper for in-browser repo browsing.
 *
 * Uses LightningFS (IndexedDB-backed) for persistence and NIP-98 auth
 * for the relay's smart HTTP git transport.
 */

// isomorphic-git expects a global Buffer (Node API) for pack-file parsing,
// tree serialization, etc. The `buffer` package (feross/buffer) is the
// standard browser polyfill — we install it before any git imports run.
import { Buffer } from "buffer";
if (typeof (globalThis as Record<string, unknown>).Buffer === "undefined") {
  (globalThis as Record<string, unknown>).Buffer = Buffer;
}

import LightningFS from "@isomorphic-git/lightning-fs";
import {
  clone,
  fetch,
  log,
  readBlob,
  readTree,
  resolveRef,
} from "isomorphic-git";
import http from "isomorphic-git/http/web";
import { makeNip98AuthHeader } from "@/shared/lib/nip98";
import { relayHttpBaseUrl } from "@/shared/lib/relay-url";

/** Get a repo-specific LightningFS instance backed by IndexedDB. */
export function getFs(owner: string, repoName: string): LightningFS {
  return new LightningFS(`buzz-git-${owner}-${repoName}`);
}

/** Working directory inside the virtual FS. */
export function getDir(owner: string, repoName: string): string {
  return `/${owner}/${repoName}`;
}

function repoGitUrl(owner: string, repoName: string): string {
  return `${relayHttpBaseUrl()}/git/${owner}/${repoName}.git`;
}

/**
 * The NIP-98 `u` tag URL — must match what transport.rs expects after
 * stripping `/info/refs`, `/git-upload-pack`, `/git-receive-pack`.
 * That means the full path including `.git`.
 */
function repoAuthUrl(owner: string, repoName: string): string {
  return `${relayHttpBaseUrl()}/git/${owner}/${repoName}.git`;
}

function authHeaders(owner: string, repoName: string): Record<string, string> {
  return {
    Authorization: makeNip98AuthHeader(repoAuthUrl(owner, repoName), "GET"),
  };
}

/**
 * Ensure a shallow clone exists in IndexedDB. If it already exists, fetch
 * the latest for the given ref.
 */
export async function ensureClone(
  owner: string,
  repoName: string,
  ref: string,
): Promise<{ fs: LightningFS; dir: string }> {
  const fs = getFs(owner, repoName);
  const dir = getDir(owner, repoName);
  const url = repoGitUrl(owner, repoName);
  const headers = authHeaders(owner, repoName);

  let exists = false;
  try {
    await fs.promises.stat(`${dir}/.git`);
    exists = true;
  } catch {
    // repo not cloned yet
  }

  if (exists) {
    try {
      await fetch({
        fs,
        http,
        dir,
        url,
        ref,
        depth: 1,
        singleBranch: true,
        headers,
      });
    } catch {
      // fetch may fail if ref hasn't changed — that's fine
    }
  } else {
    await clone({
      fs,
      http,
      dir,
      url,
      ref,
      depth: 1,
      singleBranch: true,
      noTags: true,
      headers,
    });
  }

  return { fs, dir };
}

export interface TreeEntry {
  name: string;
  type: "blob" | "tree";
  mode: string;
  oid: string;
}

/** Read tree entries at a given path (or root if no filepath). */
export async function readTreeEntries(
  fs: LightningFS,
  dir: string,
  oid: string,
  filepath?: string,
): Promise<TreeEntry[]> {
  const result = await readTree({ fs, dir, oid, filepath });
  return result.tree.map((entry) => ({
    name: entry.path,
    type: entry.type as "blob" | "tree",
    mode: entry.mode,
    oid: entry.oid,
  }));
}

export interface FileContent {
  content: string;
  isBinary: boolean;
}

/** Read a blob and decode as text. Detects binary by checking for NUL bytes. */
export async function readFileContent(
  fs: LightningFS,
  dir: string,
  oid: string,
  filepath: string,
): Promise<FileContent> {
  const { blob } = await readBlob({ fs, dir, oid, filepath });

  const checkLength = Math.min(blob.length, 512);
  for (let i = 0; i < checkLength; i++) {
    if (blob[i] === 0) {
      return { content: "", isBinary: true };
    }
  }

  const content = new TextDecoder().decode(blob);
  return { content, isBinary: false };
}

/**
 * Inline-preview caps. Different ceilings per kind:
 * - Text: a 1 MiB string is already big to render in the DOM. Over → download.
 * - Image: raster decoders handle this cheaply; cap is a sanity ceiling, not
 *   a perf brake. Normal screenshots (≤ ~few MB) preview just fine.
 * - Binary: no preview cap — we always offer download, regardless of size.
 *
 * The clone in IndexedDB always holds full bytes; these are display caps only.
 */
export const TEXT_PREVIEW_LIMIT_BYTES = 1 * 1024 * 1024;
export const IMAGE_PREVIEW_LIMIT_BYTES = 10 * 1024 * 1024;

/**
 * Discriminated view of a blob, suitable for rendering. The viewer component
 * is responsible for `URL.createObjectURL` / `revokeObjectURL` over `bytes` —
 * we deliberately do NOT create object URLs inside the React Query cache.
 *
 * Image-by-extension is restricted to raster formats. SVG is intentionally
 * absent: it can carry active content; we render SVG via the text path
 * (where applicable) instead.
 *
 * `too-large` carries the cap that was hit, so the viewer can explain which
 * limit applied without re-computing it.
 */
export type BlobView =
  | { kind: "text"; content: string; sizeBytes: number }
  | { kind: "markdown"; content: string; sizeBytes: number }
  | { kind: "html"; content: string; sizeBytes: number }
  | { kind: "image"; bytes: Uint8Array; contentType: string; sizeBytes: number }
  | { kind: "binary"; bytes: Uint8Array; sizeBytes: number }
  | {
      kind: "too-large";
      bytes: Uint8Array;
      sizeBytes: number;
      limitBytes: number;
    };

const RASTER_IMAGE_MIME: Readonly<Record<string, string>> = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  gif: "image/gif",
  webp: "image/webp",
  avif: "image/avif",
};

const MARKDOWN_EXTS = new Set(["md", "markdown"]);
const HTML_EXTS = new Set(["html", "htm"]);

function extOf(filepath: string): string {
  const base = filepath.split("/").pop() ?? "";
  const dot = base.lastIndexOf(".");
  if (dot <= 0) return "";
  return base.slice(dot + 1).toLowerCase();
}

function hasNulByte(bytes: Uint8Array): boolean {
  const n = Math.min(bytes.length, 512);
  for (let i = 0; i < n; i++) {
    if (bytes[i] === 0) return true;
  }
  return false;
}

/**
 * Classify a blob into a `BlobView`. Applies per-kind preview caps.
 *
 * Order: image-by-extension first (so a 2 MiB PNG isn't rejected as oversized
 * text), then binary detection, then text/markdown decode.
 */
export async function readBlobView(
  fs: LightningFS,
  dir: string,
  oid: string,
  filepath: string,
): Promise<BlobView> {
  const { blob } = await readBlob({ fs, dir, oid, filepath });
  const bytes = blob as Uint8Array;
  const sizeBytes = bytes.length;
  const ext = extOf(filepath);

  const mime = RASTER_IMAGE_MIME[ext];
  if (mime) {
    if (sizeBytes > IMAGE_PREVIEW_LIMIT_BYTES) {
      return {
        kind: "too-large",
        bytes,
        sizeBytes,
        limitBytes: IMAGE_PREVIEW_LIMIT_BYTES,
      };
    }
    return { kind: "image", bytes, contentType: mime, sizeBytes };
  }

  if (hasNulByte(bytes)) {
    // Binary: no preview cap. Always download.
    return { kind: "binary", bytes, sizeBytes };
  }

  if (sizeBytes > TEXT_PREVIEW_LIMIT_BYTES) {
    return {
      kind: "too-large",
      bytes,
      sizeBytes,
      limitBytes: TEXT_PREVIEW_LIMIT_BYTES,
    };
  }
  // Fatal decode: anything that *looks* binary-ish but slipped past the NUL
  // sniff (rare-but-real for non-UTF formats without an early 0x00) falls
  // through to the binary path instead of being rendered as mojibake.
  let content: string;
  try {
    content = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    return { kind: "binary", bytes, sizeBytes };
  }
  if (MARKDOWN_EXTS.has(ext)) {
    return { kind: "markdown", content, sizeBytes };
  }
  if (HTML_EXTS.has(ext)) {
    return { kind: "html", content, sizeBytes };
  }
  return { kind: "text", content, sizeBytes };
}

/**
 * Inline a repo's same-repo relative assets into one self-contained HTML
 * string, suitable for rendering inside a sandboxed iframe (which has no
 * notion of the repo's directory tree and cannot fetch siblings).
 *
 * Only *relative* `<script src>`, `<link href>`, and `<img src>` are
 * resolved — against the HTML file's own directory, scoped to paths that
 * exist in the clone. Absolute paths (`/x`) and external URLs
 * (`http(s):`, `data:`, `//host`, `#frag`) are left untouched: we never
 * reach outside the repo or rewrite something the author meant for the
 * network.
 *
 * Assets are inlined as `data:` URLs so the result is fully detached — it
 * carries no live `blob:` handles that would need revoking. This is a
 * display transform on a copy; the clone in IndexedDB is unchanged.
 */
const ASSET_MIME: Readonly<Record<string, string>> = {
  ...RASTER_IMAGE_MIME,
  js: "text/javascript",
  mjs: "text/javascript",
  css: "text/css",
  json: "application/json",
  svg: "image/svg+xml",
  woff: "font/woff",
  woff2: "font/woff2",
};

function isExternalRef(ref: string): boolean {
  // Absolute path, protocol URL, protocol-relative, fragment, or empty.
  return (
    ref === "" ||
    ref.startsWith("/") ||
    ref.startsWith("#") ||
    ref.startsWith("data:") ||
    /^[a-z][a-z0-9+.-]*:/i.test(ref) ||
    ref.startsWith("//")
  );
}

/** Resolve `dir`-relative `ref` (e.g. `../js/app.js`) to a clone path. */
function resolveRelative(baseDir: string, ref: string): string | null {
  const clean = ref.split(/[?#]/)[0];
  const parts = baseDir ? baseDir.split("/") : [];
  for (const seg of clean.split("/")) {
    if (seg === "" || seg === ".") continue;
    if (seg === "..") {
      if (parts.length === 0) return null; // escapes repo root
      parts.pop();
    } else {
      parts.push(seg);
    }
  }
  return parts.join("/");
}

function bytesToDataUrl(bytes: Uint8Array, mime: string): string {
  let binary = "";
  for (let i = 0; i < bytes.length; i++) {
    binary += String.fromCharCode(bytes[i]);
  }
  return `data:${mime};base64,${btoa(binary)}`;
}

export async function resolveHtmlAssets(
  fs: LightningFS,
  dir: string,
  oid: string,
  htmlPath: string,
  html: string,
): Promise<string> {
  const slash = htmlPath.lastIndexOf("/");
  const baseDir = slash >= 0 ? htmlPath.slice(0, slash) : "";

  const doc = new DOMParser().parseFromString(html, "text/html");

  const targets: Array<{ el: Element; attr: string }> = [
    ...[...doc.querySelectorAll("script[src]")].map((el) => ({
      el,
      attr: "src",
    })),
    ...[...doc.querySelectorAll("link[href]")].map((el) => ({
      el,
      attr: "href",
    })),
    ...[...doc.querySelectorAll("img[src]")].map((el) => ({ el, attr: "src" })),
  ];

  await Promise.all(
    targets.map(async ({ el, attr }) => {
      const ref = el.getAttribute(attr);
      if (!ref || isExternalRef(ref)) return;
      const path = resolveRelative(baseDir, ref);
      if (!path) return;
      const mime = ASSET_MIME[extOf(path)] ?? "application/octet-stream";
      try {
        const { blob } = await readBlob({ fs, dir, oid, filepath: path });
        el.setAttribute(attr, bytesToDataUrl(blob as Uint8Array, mime));
      } catch {
        // Sibling not in the clone (e.g. a deferred subtree): leave the
        // reference as-is. It will simply fail to load in the sandbox.
      }
    }),
  );

  return `<!doctype html>\n${doc.documentElement.outerHTML}`;
}

export interface CommitInfo {
  oid: string;
  message: string;
  author: {
    name: string;
    email: string;
    timestamp: number;
  };
}

/** Get recent commits for a ref. */
export async function getCommitLog(
  fs: LightningFS,
  dir: string,
  ref: string,
  depth = 20,
): Promise<CommitInfo[]> {
  const commits = await log({ fs, dir, ref, depth });
  return commits.map((c) => ({
    oid: c.oid,
    message: c.commit.message,
    author: {
      name: c.commit.author.name,
      email: c.commit.author.email,
      timestamp: c.commit.author.timestamp,
    },
  }));
}

export interface ReadmeResult {
  filename: string;
  content: string;
}

const README_PATTERNS = ["readme.md", "readme", "readme.rst", "readme.txt"];

/** Find and read a README file from the root tree. */
export async function findReadme(
  fs: LightningFS,
  dir: string,
  ref: string,
): Promise<ReadmeResult | null> {
  const oid = await resolveRef({ fs, dir, ref });
  const entries = await readTreeEntries(fs, dir, oid);

  for (const pattern of README_PATTERNS) {
    const entry = entries.find(
      (e) => e.type === "blob" && e.name.toLowerCase() === pattern,
    );
    if (entry) {
      const file = await readFileContent(fs, dir, oid, entry.name);
      if (!file.isBinary) {
        return { filename: entry.name, content: file.content };
      }
    }
  }

  return null;
}
