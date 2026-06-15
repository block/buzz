/**
 * Renders a single repo blob fetched via `useGitBlob`. Designed to be safe by
 * construction: no JS/HTML execution path, no SVG-as-image (SVG can carry
 * active content; we render it as text instead), and a hard preview-size cap
 * with a download fallback for anything over the limit.
 *
 * Object URLs for image/binary are created in a local effect and revoked on
 * unmount or input change — they are never cached inside React Query results.
 */
import { ArrowLeft, Check, Copy, Download, FileText, Play } from "lucide-react";
import { useEffect, useState } from "react";
import { Link, useParams } from "@tanstack/react-router";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { toast } from "sonner";

import { Button } from "@/shared/ui/button";
import type { BlobView } from "../git-client";
import { useGitBlob, useGitHtmlDoc } from "../use-git-browse";
import { useRepoContext } from "../use-repo-context";

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KiB`;
  return `${(n / (1024 * 1024)).toFixed(2)} MiB`;
}

function basename(path: string): string {
  return path.split("/").pop() ?? path;
}

/**
 * Stable object-URL for a byte buffer. Revokes on dependency change / unmount.
 * The viewer creates one per render-lifetime — the cache layer only stores bytes.
 */
function useObjectUrl(
  bytes: Uint8Array | null,
  contentType: string,
): string | null {
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    if (!bytes) {
      setUrl(null);
      return;
    }
    // The cast normalises `Uint8Array<ArrayBufferLike>` (isomorphic-git's
    // return shape) to `Uint8Array<ArrayBuffer>` so it's accepted as a `BlobPart`
    // under strict TS lib types.
    const blob = new Blob([bytes as Uint8Array<ArrayBuffer>], {
      type: contentType,
    });
    const next = URL.createObjectURL(blob);
    setUrl(next);
    return () => {
      URL.revokeObjectURL(next);
    };
  }, [bytes, contentType]);
  return url;
}

function CopyTextButton({ content }: { content: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <Button
      variant="outline"
      size="sm"
      onClick={async () => {
        try {
          await navigator.clipboard.writeText(content);
          setCopied(true);
          toast.success("Copied to clipboard");
          setTimeout(() => setCopied(false), 2000);
        } catch {
          toast.error("Failed to copy to clipboard");
        }
      }}
    >
      {copied ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
      <span className="ml-2">Copy</span>
    </Button>
  );
}

function DownloadButton({
  bytes,
  contentType,
  filename,
}: {
  bytes: Uint8Array;
  contentType: string;
  filename: string;
}) {
  const url = useObjectUrl(bytes, contentType);
  if (!url) return null;
  return (
    <Button asChild variant="outline" size="sm">
      <a href={url} download={filename}>
        <Download className="h-4 w-4" />
        <span className="ml-2">Download</span>
      </a>
    </Button>
  );
}

function TextView({ content }: { content: string }) {
  // Plain monospace render. Line numbers would be nice but require a list of
  // keyed children for an immutable text dump; not worth the linter dance for
  // v1. The browser handles wrapping/scrolling via `<pre>`.
  return (
    <pre className="overflow-auto whitespace-pre rounded-lg border border-border bg-muted/30 p-4 font-mono text-sm leading-6">
      {content}
    </pre>
  );
}

function ImageView({
  bytes,
  contentType,
  filename,
}: {
  bytes: Uint8Array;
  contentType: string;
  filename: string;
}) {
  const url = useObjectUrl(bytes, contentType);
  if (!url) return null;
  return (
    <div className="flex justify-center rounded-lg border border-border bg-muted/30 p-4">
      <img
        src={url}
        alt={filename}
        className="max-h-[80vh] max-w-full object-contain"
      />
    </div>
  );
}

/**
 * Runs a repo's HTML in a sandboxed iframe.
 *
 * SECURITY — the entire trust boundary is the `sandbox` attribute below.
 * `allow-scripts` lets the page's JS run; the deliberate ABSENCE of
 * `allow-same-origin` forces the frame to an opaque (`null`) origin, so its
 * scripts CANNOT read the parent's cookies, IndexedDB, localStorage, relay
 * session, or NIP-98 auth — even though we render on the same document origin.
 * Do not add `allow-same-origin`: that would hand pushed code the user's
 * session. `srcDoc` carries the asset-inlined doc; nothing reaches the network
 * for same-repo content.
 */
const RUN_SANDBOX = "allow-scripts";

function HtmlRunView({ doc }: { doc: string }) {
  return (
    <iframe
      title="Repository page (sandboxed)"
      srcDoc={doc}
      sandbox={RUN_SANDBOX}
      className="h-[80vh] w-full rounded-lg border border-border bg-white"
    />
  );
}

function ViewerBody({
  view,
  filename,
  htmlDoc,
}: {
  view: BlobView;
  filename: string;
  htmlDoc: string | null;
}) {
  switch (view.kind) {
    case "text":
      return <TextView content={view.content} />;
    case "markdown":
      return (
        <div className="prose prose-sm dark:prose-invert max-w-none rounded-lg border border-border p-4">
          <Markdown remarkPlugins={[remarkGfm]}>{view.content}</Markdown>
        </div>
      );
    case "html":
      // `htmlDoc` is the asset-inlined doc, present only once the user opts in
      // via "Run"; until then (and while it resolves) we show the source.
      return htmlDoc !== null ? (
        <HtmlRunView doc={htmlDoc} />
      ) : (
        <TextView content={view.content} />
      );
    case "image":
      return (
        <ImageView
          bytes={view.bytes}
          contentType={view.contentType}
          filename={filename}
        />
      );
    case "binary":
      return (
        <div className="rounded-lg border border-border bg-muted/30 p-6 text-sm text-muted-foreground">
          Binary file — {formatBytes(view.sizeBytes)}. Use the Download button
          above to save it.
        </div>
      );
    case "too-large":
      return (
        <div className="rounded-lg border border-border bg-muted/30 p-6 text-sm text-muted-foreground">
          File is {formatBytes(view.sizeBytes)}, over the{" "}
          {formatBytes(view.limitBytes)} preview limit. Use the Download button
          above to save it.
        </div>
      );
  }
}

export function RepoBlobPage() {
  const { repoId, _splat } = useParams({ from: "/repos/$repoId/blob/$" });
  const filepath = _splat ?? "";
  const {
    owner,
    repoName,
    defaultRef,
    isLoading: ctxLoading,
    error: ctxError,
  } = useRepoContext(repoId);

  const {
    data: view,
    isLoading,
    error,
  } = useGitBlob(owner, repoName, defaultRef, filepath);

  const [running, setRunning] = useState(false);
  const isHtml = view?.kind === "html";
  const { data: htmlDoc, isFetching: htmlFetching } = useGitHtmlDoc(
    owner,
    repoName,
    defaultRef,
    filepath,
    isHtml ? view.content : "",
    running && isHtml,
  );

  const filename = basename(filepath);

  if (ctxError) {
    return (
      <div className="px-4 py-8">
        <BackLink repoId={repoId} />
        <p className="mt-4 text-sm text-destructive">
          Failed to load repository: {ctxError.message}
        </p>
      </div>
    );
  }

  return (
    <div className="px-4 py-8">
      <BackLink repoId={repoId} />

      <div className="mt-4 flex flex-wrap items-center gap-3">
        <FileText className="h-4 w-4 text-muted-foreground" />
        <h1 className="min-w-0 truncate font-mono text-sm">{filepath}</h1>
        <div className="ml-auto flex items-center gap-2">
          {view &&
            (view.kind === "text" ||
              view.kind === "markdown" ||
              view.kind === "html") && (
              <CopyTextButton content={view.content} />
            )}
          {isHtml && (
            <Button
              variant={running ? "secondary" : "default"}
              size="sm"
              onClick={() => setRunning((r) => !r)}
            >
              <Play className="h-4 w-4" />
              <span className="ml-2">{running ? "Show source" : "Run"}</span>
            </Button>
          )}
          {view &&
            view.kind !== "text" &&
            view.kind !== "markdown" &&
            view.kind !== "html" && (
              <DownloadButton
                bytes={view.bytes}
                contentType={
                  view.kind === "image"
                    ? view.contentType
                    : "application/octet-stream"
                }
                filename={filename}
              />
            )}
        </div>
      </div>

      <div className="mt-6">
        {ctxLoading || isLoading ? (
          <div className="h-32 animate-pulse rounded-lg bg-muted" />
        ) : error ? (
          <p className="text-sm text-destructive">
            Failed to load file: {(error as Error).message}
          </p>
        ) : view ? (
          <ViewerBody
            view={view}
            filename={filename}
            htmlDoc={running && !htmlFetching ? (htmlDoc ?? null) : null}
          />
        ) : null}
      </div>
    </div>
  );
}

function BackLink({ repoId }: { repoId: string }) {
  return (
    <Link
      to="/repos/$repoId"
      params={{ repoId }}
      className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
    >
      <ArrowLeft className="h-4 w-4" />
      Back to repository
    </Link>
  );
}
