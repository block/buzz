import { useQuery } from "@tanstack/react-query";
import { useState, type ReactNode } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { api, fmtTime, shortId } from "../api";

type Props = {
  fold: string;
  version: number;
  onVersion: (v: number) => void;
};

/** Renders one artifact version: the model's free-form response with citation
 * chips, a version scrubber, and a pinned-evidence rail. */
export function ArtifactReader({ fold, version, onVersion }: Props) {
  const [pinned, setPinned] = useState<string[]>([]);

  const chain = useQuery({
    queryKey: ["artifacts", fold],
    queryFn: () => api.artifacts(fold),
  });
  const artifact = useQuery({
    queryKey: ["artifact", fold, version],
    queryFn: () => api.artifact(fold, version),
  });

  const pin = (id: string) =>
    setPinned((p) => (p.includes(id) ? p : [id, ...p]));

  const a = artifact.data;

  return (
    <div>
      <div className="toolbar">
        <strong className="mono">{fold}</strong>
        <div className="version-strip">
          {chain.data?.artifacts.map((s) => (
            <button
              key={s.version}
              className={s.version === version ? "active" : ""}
              onClick={() => onVersion(s.version)}
              title={`${fmtTime(s.created_at)} · ${s.shown} shown${
                s.truncated ? " · chunked" : ""
              }`}
            >
              v{s.version}
            </button>
          ))}
        </div>
      </div>
      {a && (
        <div className="muted" style={{ fontSize: 12, marginBottom: 10 }}>
          {fmtTime(a.created_at)} · model {a.model} · folded{" "}
          {a.shown_ids.length} new event(s)
          {a.truncated ? " · chunked (more remained pending)" : ""}
          {a.coverage_since !== null && a.coverage_until !== null && (
            <>
              {" "}
              · this run's new events span {fmtTime(a.coverage_since)} →{" "}
              {fmtTime(a.coverage_until)}
            </>
          )}
          {a.version > 1 && (
            <>
              {" "}
              · built on v{a.version - 1} (earlier versions' events aren't
              re-read — they ride along in the prior version)
            </>
          )}
        </div>
      )}
      {artifact.isError && (
        <div className="error-box">{(artifact.error as Error).message}</div>
      )}
      <div className="artifact">
        <div className="artifact-body">
          {a ? renderOutput(a.output, pin) : <span className="faint">loading…</span>}
        </div>
        <div className="evidence">
          <h2>Evidence</h2>
          {pinned.length === 0 && (
            <div className="faint" style={{ fontSize: 12 }}>
              Click a citation chip to pin the source message here — every
              claim is one click from its evidence.
            </div>
          )}
          {pinned.map((id) => (
            <EvidenceCard
              key={id}
              id={id}
              onClose={() => setPinned((p) => p.filter((x) => x !== id))}
            />
          ))}
        </div>
      </div>
    </div>
  );
}

function EvidenceCard({ id, onClose }: { id: string; onClose: () => void }) {
  const ev = useQuery({
    queryKey: ["event", id],
    queryFn: () => api.event(id),
    retry: false,
  });
  return (
    <div className="evidence-card">
      <div className="head">
        <span>
          {ev.data ? (
            <>
              <strong>{ev.data.author_name ?? shortId(ev.data.pubkey)}</strong>{" "}
              · {fmtTime(ev.data.created_at)}
            </>
          ) : (
            <span className="mono">{shortId(id)}…</span>
          )}
        </span>
        <button onClick={onClose} style={{ padding: "0 6px" }}>
          ×
        </button>
      </div>
      <div className="body">
        {ev.isError
          ? `not in the mirror: ${(ev.error as Error).message}`
          : (ev.data?.content ?? "loading…")}
      </div>
    </div>
  );
}

const CITATION = /\[event:([0-9a-f]{64})\]/g;
const CITE_HREF = "#cite:";

/** Real markdown rendering (GFM) for the artifact's output. `[event:<id>]`
 * citations are pre-rewritten into `#cite:` links, which the `a` component
 * turns into evidence chips; H1/H2 downshift to fit the pane's hierarchy. */
function renderOutput(output: string, pin: (id: string) => void): ReactNode {
  const withChips = output.replace(
    CITATION,
    (_, id: string) => `[${shortId(id)}](${CITE_HREF}${id})`,
  );
  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      components={{
        h1: ({ children }) => <h3>{children}</h3>,
        h2: ({ children }) => <h4>{children}</h4>,
        a: ({ href, children }) => {
          if (href?.startsWith(CITE_HREF)) {
            const id = href.slice(CITE_HREF.length);
            return (
              <button className="chip" title={id} onClick={() => pin(id)}>
                {children}
              </button>
            );
          }
          return (
            <a href={href} target="_blank" rel="noreferrer">
              {children}
            </a>
          );
        },
      }}
    >
      {withChips}
    </ReactMarkdown>
  );
}
