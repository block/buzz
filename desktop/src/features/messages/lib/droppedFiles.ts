/**
 * OS file-manager drops into the Tauri/WebKit composer.
 *
 * Linux WebKitGTK (and some other webviews) often omit `dataTransfer.files`
 * and instead deliver `text/uri-list` / `text/plain` carrying `file://` URIs
 * or absolute paths. ProseMirror then inserts that path as chat text unless
 * `editorProps.handleDrop` claims the event.
 *
 * WebKitGTK also sometimes fills `files` with dummy `File` objects (`size`
 * 0, empty name). Those must not win over URI-list paths, or the drop is
 * claimed and then silently discarded. On GNOME Files + Wayland even the
 * URI-list is empty; the GTK `os-file-drop` event supplies the real paths.
 */

export type DroppedFilePayload = {
  files: File[];
  /** Absolute filesystem paths when the webview did not populate `File` objects. */
  paths: string[];
};

/** MIME types that mean "this drag is files", not in-app text. */
const FILE_DRAG_MIME_TYPES = [
  "Files",
  "text/uri-list",
  "text/x-moz-url",
  "x-special/gnome-copied-files",
  "application/x-kde4-urilist",
] as const;

/** Extra MIME types that may carry paths on drop. `text/plain` is not a
 *  drag signal — in-app text/mention drags use it. */
const PATH_MIME_TYPES = [
  ...FILE_DRAG_MIME_TYPES.filter((type) => type !== "Files"),
  "text/plain",
] as const;

function unique(values: string[]): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const value of values) {
    if (seen.has(value)) continue;
    seen.add(value);
    out.push(value);
  }
  return out;
}

/** Convert a `file://` URI or OS absolute path into a local path, or null. */
export function fileUriOrAbsolutePath(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed) return null;

  if (/^https?:\/\//i.test(trimmed)) return null;

  // Require `file:/` so `file:not-a-url` is not parsed as `/not-a-url`.
  if (/^file:\//i.test(trimmed)) {
    try {
      const url = new URL(trimmed);
      if (url.protocol !== "file:") return null;
      const host = url.hostname.toLowerCase();
      if (host && host !== "localhost") return null;
      let path = decodeURIComponent(url.pathname);
      // `file:///C:/Users/...` → `C:/Users/...`
      if (/^\/[A-Za-z]:\//.test(path)) path = path.slice(1);
      return path;
    } catch {
      const withoutScheme = trimmed.replace(/^file:/i, "");
      if (withoutScheme.startsWith("/")) {
        try {
          return decodeURIComponent(withoutScheme);
        } catch {
          return withoutScheme;
        }
      }
      return null;
    }
  }

  // Unix absolute, Windows drive, Windows UNC.
  if (trimmed.startsWith("/") && !trimmed.startsWith("//")) return trimmed;
  if (/^[A-Za-z]:[\\/]/.test(trimmed)) return trimmed;
  if (trimmed.startsWith("\\\\")) return trimmed;
  return null;
}

/** Parse `text/uri-list` or newline-separated plain text into local paths. */
export function extractPathsFromText(text: string): string[] {
  if (!text) return [];
  const lines = text
    .replace(/\r\n/g, "\n")
    .replace(/\0/g, "\n")
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0 && !line.startsWith("#"));
  const paths: string[] = [];
  for (const line of lines) {
    const path = fileUriOrAbsolutePath(line);
    if (path) paths.push(path);
  }
  return unique(paths);
}

function mimeTypes(data: DataTransfer): string[] {
  return Array.from(data.types ?? []);
}

/**
 * True when the drag payload is an OS file drop (not in-app text/mention
 * drag). Used for overlay + preventDefault on dragover, where `getData` is
 * often empty until drop.
 */
export function isOsFileDrag(data: DataTransfer | null | undefined): boolean {
  if (!data) return false;
  const types = mimeTypes(data);
  return FILE_DRAG_MIME_TYPES.some((type) => types.includes(type));
}

/** Mark the drag as a copy so WebKitGTK actually fires `drop`. */
export function markOsFileDragOver(data: DataTransfer | null | undefined): void {
  if (!data || !isOsFileDrag(data)) return;
  try {
    data.dropEffect = "copy";
  } catch {
    // Some webviews freeze dropEffect during dragover.
  }
}

function isUsableDroppedFile(file: File | null | undefined): file is File {
  return Boolean(file && file.size > 0 && file.name);
}

function filesFromList(list: ArrayLike<File> | null | undefined): File[] {
  return Array.from(list ?? []).filter(isUsableDroppedFile);
}

function filesFromItems(data: DataTransfer): File[] {
  const items = data.items;
  if (!items) return [];
  const files: File[] = [];
  try {
    for (let index = 0; index < items.length; index++) {
      const item = items[index];
      if (!item || item.kind !== "file") continue;
      const file = item.getAsFile();
      if (isUsableDroppedFile(file)) files.push(file);
    }
  } catch {
    // WebKitGTK can throw when enumerating items after the drop event.
  }
  return files;
}

function extractPathsFromDataTransfer(data: DataTransfer): string[] {
  const types = unique([...mimeTypes(data), ...PATH_MIME_TYPES]);
  const paths: string[] = [];
  for (const type of types) {
    if (type === "Files") continue;
    paths.push(...extractPathsFromText(safeGetData(data, type)));
  }
  return unique(paths).filter(looksLikeFileName);
}

function uniqueFiles(files: File[]): File[] {
  const seen = new Set<string>();
  const out: File[] = [];
  for (const file of files) {
    const key = `${file.name}:${file.size}:${file.type}`;
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(file);
  }
  return out;
}

/**
 * Prefer real `File` objects when the webview populated them. Otherwise
 * recover absolute paths from URI-list / plain text so Linux drops still
 * attach instead of inserting a path string into the composer.
 *
 * Dummy `File` entries (`size` 0) are ignored so URI-list paths still win.
 */
export function extractDroppedFilePayload(
  data: DataTransfer | null | undefined,
): DroppedFilePayload {
  if (!data) return { files: [], paths: [] };
  const files = uniqueFiles([
    ...filesFromList(data.files),
    ...filesFromItems(data),
  ]);
  if (files.length > 0) return { files, paths: [] };
  return { files: [], paths: extractPathsFromDataTransfer(data) };
}

export function basenameFromPath(path: string): string {
  const parts = path.split(/[/\\]/);
  const last = parts.at(-1)?.trim();
  return last && last.length > 0 ? last : "file";
}

/** Skip path-like chat text (`/usr/bin/env python`) that is not a dropped file. */
export function looksLikeFileName(path: string): boolean {
  const base = basenameFromPath(path);
  const dot = base.lastIndexOf(".");
  return dot > 0 && dot < base.length - 1;
}

function safeGetData(data: DataTransfer, type: string): string {
  try {
    return data.getData(type) ?? "";
  } catch {
    return "";
  }
}
