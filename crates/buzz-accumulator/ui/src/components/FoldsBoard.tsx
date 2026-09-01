import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import {
  api,
  fmtTime,
  fmtTokens,
  selectionIsRunnable,
  type FoldRow,
  type Preflight,
  type RunOutcome,
  type Selection,
} from "../api";

type Props = {
  selection: Selection;
  onOpenArtifact: (fold: string, version: number) => void;
};

export function FoldsBoard({ selection, onOpenArtifact }: Props) {
  const folds = useQuery({ queryKey: ["folds"], queryFn: api.folds });

  return (
    <div>
      <h2>Folds</h2>
      <CreateFold selection={selection} />
      {folds.isError && (
        <div className="error-box">{(folds.error as Error).message}</div>
      )}
      {folds.data?.folds.length === 0 && (
        <div className="faint">
          No folds yet. Compose a selection, then create one above — runs are
          always priced before they spend.
        </div>
      )}
      {folds.data?.folds.map((f) => (
        <FoldCard key={f.name} fold={f} onOpenArtifact={onOpenArtifact} />
      ))}
    </div>
  );
}

function CreateFold({ selection }: { selection: Selection }) {
  const qc = useQueryClient();
  const [name, setName] = useState("");
  const [model, setModel] = useState("haiku");
  const [instructions, setInstructions] = useState("");

  const create = useMutation({
    mutationFn: () =>
      api.putFold(name.trim(), {
        selection,
        model,
        ...(instructions.trim() ? { instructions } : {}),
      }),
    onSuccess: () => {
      setName("");
      setInstructions("");
      qc.invalidateQueries({ queryKey: ["folds"] });
    },
  });

  const ready = name.trim().length > 0 && selectionIsRunnable(selection);

  return (
    <div className="create-fold">
      <div className="field">
        <label>New fold (uses the current selection)</label>
        <input
          className="mono"
          placeholder="name — [a-z0-9-], e.g. digest--2026-w36"
          value={name}
          onChange={(e) => setName(e.target.value)}
        />
      </div>
      <div className="field">
        <label>Model</label>
        <select value={model} onChange={(e) => setModel(e.target.value)}>
          <option value="haiku">haiku</option>
          <option value="sonnet">sonnet</option>
          <option value="opus">opus</option>
        </select>
      </div>
      <div className="field">
        <label>Instructions</label>
        <textarea
          rows={2}
          placeholder="task focus, e.g. 'track decisions and blockers' — the Working Context / Log output contract always applies"
          value={instructions}
          onChange={(e) => setInstructions(e.target.value)}
        />
      </div>
      <button
        className="primary"
        disabled={!ready || create.isPending}
        onClick={() => create.mutate()}
      >
        {create.isPending ? "saving…" : "create fold"}
      </button>
      {!selectionIsRunnable(selection) && (
        <span className="faint" style={{ marginLeft: 8 }}>
          needs a selection
        </span>
      )}
      {create.isError && (
        <div className="plan-box err">{(create.error as Error).message}</div>
      )}
    </div>
  );
}

function FoldCard({
  fold,
  onOpenArtifact,
}: {
  fold: FoldRow;
  onOpenArtifact: (fold: string, version: number) => void;
}) {
  const qc = useQueryClient();
  const [plan, setPlan] = useState<Preflight | null>(null);
  const [outcome, setOutcome] = useState<RunOutcome | null>(null);

  const preflight = useMutation({
    mutationFn: () => api.preflight(fold.name, {}),
    onSuccess: (p) => {
      setPlan(p);
      setOutcome(null);
    },
  });

  const run = useMutation({
    mutationFn: () => {
      if (plan?.plan !== "ready") throw new Error("preflight first");
      // Pin the run to the exact window that was priced.
      const [since, until_exclusive] = plan.window;
      return api.run(fold.name, { since, until_exclusive });
    },
    onSuccess: (o) => {
      setOutcome(o);
      setPlan(null);
      qc.invalidateQueries({ queryKey: ["folds"] });
      qc.invalidateQueries({ queryKey: ["artifacts", fold.name] });
      if (o.status === "folded") {
        onOpenArtifact(fold.name, o.artifact.version);
      }
    },
    onError: () => setPlan(null),
  });

  const sel = fold.spec.selection;
  const selDesc = [
    sel.channels.length > 0 ? `${sel.channels.length} channel(s)` : null,
    sel.authors.length > 0 ? `${sel.authors.length} author(s)` : null,
  ]
    .filter(Boolean)
    .join(" · ");

  return (
    <div className="fold-card">
      <div className="name mono">{fold.name}</div>
      <div className="desc">
        {selDesc || "—"} · {fold.spec.model} ·{" "}
        {fold.versions > 0
          ? `v${fold.latest_version} · updated ${fmtTime(fold.updated_at)}`
          : "never run"}
      </div>
      <div className="actions">
        <button
          onClick={() => preflight.mutate()}
          disabled={preflight.isPending || run.isPending}
        >
          {preflight.isPending ? "pricing…" : "preflight ($0)"}
        </button>
        <button
          className="primary"
          disabled={plan?.plan !== "ready" || run.isPending}
          onClick={() => run.mutate()}
        >
          {run.isPending ? "folding…" : "run"}
        </button>
        {fold.latest_version !== null && (
          <button onClick={() => onOpenArtifact(fold.name, fold.latest_version ?? 1)}>
            read v{fold.latest_version}
          </button>
        )}
      </div>
      {preflight.isError && (
        <div className="plan-box err">{(preflight.error as Error).message}</div>
      )}
      {run.isError && (
        <div className="plan-box err">{(run.error as Error).message}</div>
      )}
      {plan && <PlanBox plan={plan} />}
      {outcome && <OutcomeBox outcome={outcome} />}
    </div>
  );
}

function PlanBox({ plan }: { plan: Preflight }) {
  if (plan.plan === "cached") {
    return <div className="plan-box ok">Up to date — nothing new to fold.</div>;
  }
  if (plan.plan === "stalled") {
    return (
      <div className="plan-box warn">
        Stalled: {plan.reason} ({plan.pending} pending)
      </div>
    );
  }
  const est = plan.estimate;
  const fits = est.window_fit.fits;
  return (
    <div className="plan-box ok">
      Ready: {plan.shown} shown / {plan.pending} pending
      {plan.truncated ? " (chunked)" : ""} · ~
      {fmtTokens(est.est_input_tokens)} input tokens
      {fits === false ? " · OVER the model window" : ""}
      <br />
      <span className="faint">
        Run is pinned to this priced window; press run to spend.
      </span>
    </div>
  );
}

function OutcomeBox({ outcome }: { outcome: RunOutcome }) {
  switch (outcome.status) {
    case "cached":
      return <div className="plan-box ok">Already up to date.</div>;
    case "stalled":
      return (
        <div className="plan-box warn">
          Stalled: {outcome.reason} ({outcome.pending} pending)
        </div>
      );
    case "refused":
      return (
        <div className="plan-box err">
          Model output refused ({outcome.reason}). Nothing persisted; raw
          output:
          <pre className="mono" style={{ whiteSpace: "pre-wrap" }}>
            {outcome.model_output.slice(0, 2000)}
          </pre>
        </div>
      );
    case "unpublished":
      return (
        <div className="plan-box err">
          Not persisted: {outcome.reason}
        </div>
      );
    case "folded":
      return (
        <div className="plan-box ok">
          Folded v{outcome.artifact.version} — {outcome.shown} shown
          {outcome.truncated
            ? ` (chunked; ${outcome.pending - outcome.shown} remain)`
            : ""}
          .
        </div>
      );
  }
}
