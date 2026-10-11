import type * as React from "react";

type Props = {
  dim?: string;
  error: string | null;
  label: string;
  onRetry: () => void;
  placeholderRef?: React.Ref<HTMLSpanElement>;
};

function reserve(dim?: string): React.CSSProperties {
  const m = /^(\d+)x(\d+)$/.exec(dim ?? "");
  const w = m ? Number(m[1]) : 384;
  const h = m ? Number(m[2]) : 216;
  const scale = Math.min(1, 384 / w, 256 / h);
  return {
    aspectRatio: `${w} / ${h}`,
    width: `${Math.max(96, Math.round(w * scale))}px`,
  };
}

/** Sized placeholder shown while an auth-gated image loads (web) and a clear error + Retry when it fails. */
export function RelayMediaPlaceholder({
  dim,
  error,
  label,
  onRetry,
  placeholderRef,
}: Props) {
  return (
    <span
      ref={placeholderRef}
      className="flex max-w-full flex-col items-center justify-center gap-1 rounded-2xl border border-border/70 bg-muted/40 p-3 text-center text-xs text-muted-foreground"
      data-relay-media-placeholder={error ? "error" : "loading"}
      role={error ? "alert" : "status"}
      style={reserve(dim)}
    >
      {error ? (
        <>
          <span>Couldn’t load {label.toLowerCase()}</span>
          <span className="max-w-full truncate opacity-70">{error}</span>
          <button
            className="mt-1 rounded-md border border-border px-2 py-0.5 text-foreground hover:bg-muted"
            data-relay-media-retry=""
            onClick={onRetry}
            type="button"
          >
            Retry
          </button>
        </>
      ) : (
        <span className="animate-pulse">Loading {label.toLowerCase()}…</span>
      )}
    </span>
  );
}
