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
  return new LightningFS(`sprout-git-${owner}-${repoName}`);
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

  // Check first 512 bytes for NUL to detect binary
  const checkLength = Math.min(blob.length, 512);
  for (let i = 0; i < checkLength; i++) {
    if (blob[i] === 0) {
      return { content: "", isBinary: true };
    }
  }

  const content = new TextDecoder().decode(blob);
  return { content, isBinary: false };
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
