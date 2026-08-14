import * as React from "react";

import { useTheme } from "@/shared/theme/ThemeProvider";

const MAX_MERMAID_SOURCE_LENGTH = 20_000;
const MAX_MERMAID_EDGES = 200;

let mermaidModulePromise: Promise<typeof import("mermaid")> | null = null;
// Mermaid configuration is global, so keep each initialize/render pair atomic.
let renderQueue: Promise<void> = Promise.resolve();

function loadMermaid() {
  mermaidModulePromise ??= import("mermaid");
  return mermaidModulePromise;
}

function renderMermaid(renderId: string, source: string, isDark: boolean) {
  const result = renderQueue.then(async () => {
    const { default: mermaid } = await loadMermaid();
    mermaid.initialize({
      maxEdges: MAX_MERMAID_EDGES,
      maxTextSize: MAX_MERMAID_SOURCE_LENGTH,
      securityLevel: "strict",
      startOnLoad: false,
      suppressErrorRendering: true,
      theme: isDark ? "dark" : "default",
    });
    return mermaid.render(renderId, source);
  });
  renderQueue = result.then(
    () => undefined,
    () => undefined,
  );
  return result;
}

export function MermaidDiagram({
  fallback,
  source,
}: {
  fallback: React.ReactNode;
  source: string;
}) {
  const { isDark } = useTheme();
  const reactId = React.useId();
  const renderId = React.useMemo(
    () => `mermaid-${reactId.replace(/[^a-zA-Z0-9_-]/g, "")}`,
    [reactId],
  );
  const [imageUrl, setImageUrl] = React.useState<string | null>(null);
  const [failed, setFailed] = React.useState(false);
  const canRender =
    source.trim().length > 0 && source.length <= MAX_MERMAID_SOURCE_LENGTH;

  React.useEffect(() => {
    if (!canRender) return;

    let cancelled = false;
    let objectUrl: string | null = null;
    setFailed(false);
    setImageUrl(null);

    async function renderDiagram() {
      try {
        const { svg } = await renderMermaid(renderId, source, isDark);
        if (cancelled) return;

        objectUrl = URL.createObjectURL(
          new Blob([svg], { type: "image/svg+xml" }),
        );
        setImageUrl(objectUrl);
      } catch {
        if (!cancelled) setFailed(true);
      }
    }

    void renderDiagram();
    return () => {
      cancelled = true;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [canRender, isDark, renderId, source]);

  if (!canRender || failed) return fallback;

  return (
    <div
      aria-busy={!imageUrl}
      className="flex min-h-24 max-w-full items-center justify-center overflow-auto rounded-2xl border border-border/70 bg-muted/30 p-3"
      data-mermaid-diagram=""
    >
      {imageUrl ? (
        <img
          alt="Mermaid diagram"
          className="max-h-[600px] max-w-full"
          draggable={false}
          src={imageUrl}
        />
      ) : (
        <span className="text-xs text-muted-foreground" role="status">
          Rendering diagram…
        </span>
      )}
    </div>
  );
}
