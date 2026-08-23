import * as React from "react";
import { ChevronLeft, ExternalLink, Search } from "lucide-react";

import {
  getHuggingFaceModel,
  searchHuggingFaceModels,
  type HuggingFaceModelSummary,
} from "@/shared/api/tauriMesh";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Spinner } from "@/shared/ui/spinner";
import {
  formatModelBytes,
  immutableHuggingFaceModelRef,
} from "../huggingFaceModels";

const SEARCH_DEBOUNCE_MS = 350;

export function HuggingFaceModelBrowser({
  disabled,
  onSelect,
}: {
  disabled: boolean;
  onSelect: (modelRef: string) => void;
}) {
  const [open, setOpen] = React.useState(false);
  const [query, setQuery] = React.useState("");
  const [repositories, setRepositories] = React.useState<
    HuggingFaceModelSummary[]
  >([]);
  const [nextCursor, setNextCursor] = React.useState<string | null>(null);
  const [selected, setSelected] =
    React.useState<HuggingFaceModelSummary | null>(null);
  const [loading, setLoading] = React.useState(false);
  const [loadingMore, setLoadingMore] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const requestGeneration = React.useRef(0);

  const runSearch = React.useCallback(
    async (searchQuery: string, cursor?: string) => {
      const generation = ++requestGeneration.current;
      cursor ? setLoadingMore(true) : setLoading(true);
      setError(null);
      try {
        const response = await searchHuggingFaceModels({
          query: searchQuery,
          cursor,
          pageSize: 8,
        });
        if (requestGeneration.current !== generation) return;
        setRepositories((current) =>
          cursor
            ? [...current, ...response.repositories]
            : response.repositories,
        );
        setNextCursor(response.nextCursor);
      } catch (searchError) {
        if (requestGeneration.current !== generation) return;
        setError(
          searchError instanceof Error
            ? searchError.message
            : String(searchError),
        );
        if (!cursor) setRepositories([]);
      } finally {
        if (requestGeneration.current === generation) {
          setLoading(false);
          setLoadingMore(false);
        }
      }
    },
    [],
  );

  React.useEffect(() => {
    if (!open || selected) return;
    const timeout = window.setTimeout(() => {
      void runSearch(query);
    }, SEARCH_DEBOUNCE_MS);
    return () => window.clearTimeout(timeout);
  }, [open, query, runSearch, selected]);

  async function openRepository(repository: HuggingFaceModelSummary) {
    const generation = ++requestGeneration.current;
    setLoading(true);
    setError(null);
    try {
      const detail = await getHuggingFaceModel(repository.repoId);
      if (requestGeneration.current === generation) setSelected(detail);
    } catch (detailError) {
      if (requestGeneration.current !== generation) return;
      setError(
        detailError instanceof Error
          ? detailError.message
          : String(detailError),
      );
    } finally {
      if (requestGeneration.current === generation) setLoading(false);
    }
  }

  function closeBrowser() {
    requestGeneration.current += 1;
    setOpen(false);
    setSelected(null);
    setError(null);
  }

  if (!open) {
    return (
      <Button
        data-testid="mesh-huggingface-browse"
        disabled={disabled}
        onClick={() => setOpen(true)}
        size="sm"
        type="button"
        variant="outline"
      >
        Browse Hugging Face
      </Button>
    );
  }

  return (
    <section
      className="space-y-3 rounded-lg border border-input/40 bg-background/50 p-3"
      data-testid="mesh-huggingface-browser"
    >
      <div className="flex items-center justify-between gap-3">
        <div className="flex min-w-0 items-center gap-2">
          {selected ? (
            <Button
              aria-label="Back to Hugging Face search"
              onClick={() => {
                setSelected(null);
                setError(null);
              }}
              size="icon-xs"
              type="button"
              variant="ghost"
            >
              <ChevronLeft />
            </Button>
          ) : null}
          <h3 className="truncate text-sm font-medium">
            {selected?.repoId ?? "Hugging Face models"}
          </h3>
        </div>
        <Button onClick={closeBrowser} size="xs" type="button" variant="ghost">
          Close
        </Button>
      </div>

      {selected ? (
        <RepositoryFiles
          disabled={disabled}
          model={selected}
          onSelect={(file) => {
            onSelect(immutableHuggingFaceModelRef(selected, file));
            closeBrowser();
          }}
        />
      ) : (
        <>
          <div className="relative">
            <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
            <Input
              aria-label="Search Hugging Face models"
              className="pl-8"
              data-testid="mesh-huggingface-search"
              disabled={disabled}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="Search text-generation GGUF models…"
              value={query}
            />
          </div>
          {loading ? (
            <p className="flex items-center gap-2 py-3 text-sm text-muted-foreground">
              <Spinner className="h-3.5 w-3.5" /> Searching Hugging Face…
            </p>
          ) : repositories.length === 0 && !error ? (
            <p className="py-3 text-sm text-muted-foreground">
              No compatible GGUF repositories found.
            </p>
          ) : (
            <div className="max-h-64 space-y-1 overflow-y-auto">
              {repositories.map((repository) => (
                <button
                  className="flex w-full min-w-0 items-center justify-between gap-3 rounded-lg px-2.5 py-2 text-left transition-colors hover:bg-muted/60 focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
                  data-testid={`mesh-huggingface-repository-${repository.repoId}`}
                  disabled={disabled}
                  key={`${repository.repoId}@${repository.revision}`}
                  onClick={() => void openRepository(repository)}
                  type="button"
                >
                  <span className="min-w-0">
                    <span className="block truncate text-sm font-medium">
                      {repository.repoId}
                    </span>
                    <span className="block truncate text-2xs text-muted-foreground">
                      {repository.license ?? "License not specified"} ·{" "}
                      {repository.files.length} GGUF option
                      {repository.files.length === 1 ? "" : "s"}
                    </span>
                  </span>
                  {repository.gated ? (
                    <span className="shrink-0 rounded bg-amber-500/15 px-1.5 text-2xs font-medium text-amber-700 dark:text-amber-300">
                      Gated
                    </span>
                  ) : null}
                </button>
              ))}
              {nextCursor ? (
                <Button
                  className="mt-2 w-full"
                  disabled={loadingMore || disabled}
                  onClick={() => void runSearch(query, nextCursor)}
                  size="sm"
                  type="button"
                  variant="ghost"
                >
                  {loadingMore ? <Spinner className="h-3.5 w-3.5" /> : null}
                  Load more
                </Button>
              ) : null}
            </div>
          )}
        </>
      )}

      {error ? (
        <p className="text-sm text-destructive" role="alert">
          {error}
        </p>
      ) : null}
    </section>
  );
}

function RepositoryFiles({
  disabled,
  model,
  onSelect,
}: {
  disabled: boolean;
  model: HuggingFaceModelSummary;
  onSelect: (file: HuggingFaceModelSummary["files"][number]) => void;
}) {
  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-2xs text-muted-foreground">
        <span>{model.license ?? "License not specified"}</span>
        <span>{model.downloads.toLocaleString()} downloads</span>
        <a
          className="inline-flex items-center gap-1 underline underline-offset-2"
          href={model.webUrl}
          rel="noreferrer"
          target="_blank"
        >
          Model page <ExternalLink className="h-3 w-3" />
        </a>
      </div>
      {model.gated ? (
        <p className="rounded-lg bg-amber-500/10 px-2.5 py-2 text-sm text-amber-800 dark:text-amber-200">
          {model.gatedDownloadReady
            ? "This model requires accepting its access terms on Hugging Face. The HF_TOKEN used to launch Zorro must have read access."
            : "This model requires Hugging Face access. Saved-token downloads need an updated MeshLLM SDK; until then, launch Zorro with HF_TOKEN or choose a public model."}
        </p>
      ) : null}
      <div className="max-h-64 space-y-1 overflow-y-auto">
        {model.files.map((file) => {
          const size = formatModelBytes(file.sizeBytes);
          return (
            <button
              className={cn(
                "flex w-full min-w-0 items-center justify-between gap-3 rounded-lg px-2.5 py-2 text-left transition-colors",
                "hover:bg-muted/60 focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring",
              )}
              disabled={disabled || (model.gated && !model.gatedDownloadReady)}
              key={file.path}
              onClick={() => onSelect(file)}
              type="button"
            >
              <span className="min-w-0 truncate text-sm">{file.path}</span>
              <span className="shrink-0 text-2xs text-muted-foreground">
                {[file.quantization, size, file.multipart ? "multipart" : null]
                  .filter(Boolean)
                  .join(" · ")}
              </span>
            </button>
          );
        })}
      </div>
      <p className="text-2xs text-muted-foreground">
        Selection is pinned to commit {model.revision.slice(0, 12)} so a future
        repository update cannot silently change the model.
      </p>
    </div>
  );
}
