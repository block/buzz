import * as React from "react";
import {
  interpretBattleRhythmDocument,
  pickBattleRhythmDocument,
} from "@/shared/api/tauriBattleRhythm";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import type { ImportRevisionInput } from "../data/battleRhythmService";
import type {
  BattleRhythmEvent,
  BattleRhythmSource,
} from "../domain/contracts";
import {
  buildImportRevision,
  interpretExtractedDocument,
  parseImportProposal,
  type ImportProposal,
} from "../domain/importDiff";

const TIME_ZONE = "Australia/Sydney";

export function ImportReviewDialog({
  open,
  onOpenChange,
  ownerPubkey,
  coverage,
  sources,
  events,
  onApply,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  ownerPubkey: string;
  coverage: Readonly<{ start: string; end: string }>;
  sources: readonly BattleRhythmSource[];
  events: readonly BattleRhythmEvent[];
  onApply: (input: ImportRevisionInput) => Promise<void>;
}) {
  const [sourceType, setSourceType] =
    React.useState<ImportProposal["sourceType"]>("shortcast");
  const [selectedSourceId, setSelectedSourceId] = React.useState("new");
  const [proposal, setProposal] = React.useState<ImportProposal>();
  const [interpretationMode, setInterpretationMode] = React.useState<
    "model" | "deterministic"
  >();
  const [document, setDocument] =
    React.useState<Awaited<ReturnType<typeof pickBattleRhythmDocument>>>();
  const [selectedLocations, setSelectedLocations] = React.useState<Set<string>>(
    new Set(),
  );
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string>();
  const matchingSources = sources.filter(
    (source) => source.type === sourceType,
  );
  const selectedSource = matchingSources.find(
    (source) => source.id === selectedSourceId,
  );
  const filteredProposal = proposal
    ? {
        ...proposal,
        events: proposal.events.filter((event) =>
          selectedLocations.has(event.sourceLocation),
        ),
      }
    : undefined;
  const preview =
    filteredProposal && document
      ? buildImportRevision({
          sourceId: selectedSource?.id ?? "preview",
          revisionId: "preview",
          priorRevisionId: selectedSource?.revisionId ?? null,
          importedAt: new Date().toISOString(),
          timeZone: TIME_ZONE,
          proposal: filteredProposal,
          existing: events,
        })
      : undefined;

  async function chooseDocument() {
    setBusy(true);
    setError(undefined);
    try {
      const picked = await pickBattleRhythmDocument();
      if (!picked) return;
      let interpreted: ImportProposal | undefined;
      try {
        const modelResult = await interpretBattleRhythmDocument(
          picked,
          sourceType,
          coverage,
        );
        if (modelResult) {
          const parsed = parseImportProposal(modelResult);
          if (
            parsed.sourceType !== sourceType ||
            parsed.proposedCoverage.start !== coverage.start ||
            parsed.proposedCoverage.end !== coverage.end
          )
            throw new Error("Model proposal does not match the import.");
          interpreted = parsed;
          setInterpretationMode("model");
        }
      } catch {
        // The deterministic interpreter below keeps import available when a
        // configured model route is offline or returns invalid structured data.
      }
      if (!interpreted) {
        interpreted = interpretExtractedDocument(
          picked,
          sourceType,
          coverage,
          TIME_ZONE,
        );
        setInterpretationMode("deterministic");
      }
      setDocument(picked);
      setProposal(interpreted);
      setSelectedLocations(
        new Set(interpreted.events.map((event) => event.sourceLocation)),
      );
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Document import failed.",
      );
    } finally {
      setBusy(false);
    }
  }

  async function apply() {
    if (!document || !filteredProposal || !ownerPubkey) return;
    setBusy(true);
    setError(undefined);
    try {
      const importedAt = new Date().toISOString();
      const sourceId = selectedSource?.id ?? crypto.randomUUID();
      const revisionId = crypto.randomUUID();
      const result = buildImportRevision({
        sourceId,
        revisionId,
        priorRevisionId: selectedSource?.revisionId ?? null,
        importedAt,
        timeZone: TIME_ZONE,
        proposal: filteredProposal,
        existing: events,
      });
      await onApply({
        ownerPubkey,
        source: {
          schemaVersion: 1,
          id: sourceId,
          type: sourceType,
          displayName:
            selectedSource?.displayName ??
            `${sourceType === "fas" ? "Fleet Activity Schedule" : sourceType === "longcast" ? "Longcast" : "Shortcast"}`,
          coverageStart: coverage.start,
          coverageEnd: coverage.end,
          documentName: document.filename,
          documentHash: document.sha256,
          revisionId,
          priorRevisionId: selectedSource?.revisionId ?? null,
          importedAt,
          status: "approved",
          sourceReference: `local-document:${document.sha256}`,
        },
        revision: result.revision,
        events: result.events,
      });
      onOpenChange(false);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Import failed.");
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[85vh] max-w-3xl overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Import planning document</DialogTitle>
        </DialogHeader>
        <div className="grid gap-4">
          <div className="grid gap-3 sm:grid-cols-2">
            <label className="grid gap-1 text-sm">
              Planning source
              <select
                className="rounded border bg-background p-2"
                onChange={(event) => {
                  setSourceType(
                    event.target.value as ImportProposal["sourceType"],
                  );
                  setSelectedSourceId("new");
                  setDocument(undefined);
                  setProposal(undefined);
                  setInterpretationMode(undefined);
                }}
                value={sourceType}
              >
                <option value="fas">Fleet Activity Schedule</option>
                <option value="longcast">Longcast</option>
                <option value="shortcast">Shortcast</option>
              </select>
            </label>
            <label className="grid gap-1 text-sm">
              Import mode
              <select
                className="rounded border bg-background p-2"
                onChange={(event) => setSelectedSourceId(event.target.value)}
                value={selectedSourceId}
              >
                <option value="new">New source</option>
                {matchingSources.map((source) => (
                  <option key={source.id} value={source.id}>
                    Revise {source.displayName}
                  </option>
                ))}
              </select>
            </label>
          </div>
          <button
            className="w-fit rounded bg-primary px-3 py-2 text-sm text-primary-foreground disabled:opacity-50"
            disabled={busy}
            onClick={chooseDocument}
            type="button"
          >
            {busy ? "Reading document…" : "Choose Word, Excel, or PDF"}
          </button>
          {document && proposal ? (
            <>
              <div className="rounded border p-3 text-sm">
                <strong>{document.filename}</strong>
                <p className="text-xs text-muted-foreground">
                  {document.blocks.length} extracted entries ·{" "}
                  {proposal.events.length} dated calendar entries proposed ·{" "}
                  {interpretationMode === "model"
                    ? "Model-assisted interpretation"
                    : "Deterministic interpretation"}
                </p>
              </div>
              {proposal.uncertainties.length > 0 ? (
                <div className="rounded border border-amber-500/50 bg-amber-500/10 p-3">
                  <strong className="text-sm">Review notes</strong>
                  <ul className="mt-1 list-disc pl-5 text-xs">
                    {proposal.uncertainties.map((item) => (
                      <li key={`${item.location}:${item.message}`}>
                        {item.location}: {item.message}
                      </li>
                    ))}
                  </ul>
                </div>
              ) : null}
              <div className="grid max-h-72 gap-2 overflow-y-auto">
                {proposal.events.map((event) => (
                  <label
                    className="flex gap-3 rounded border p-3 text-sm"
                    key={event.sourceLocation}
                  >
                    <input
                      checked={selectedLocations.has(event.sourceLocation)}
                      onChange={(change) => {
                        const next = new Set(selectedLocations);
                        if (change.target.checked)
                          next.add(event.sourceLocation);
                        else next.delete(event.sourceLocation);
                        setSelectedLocations(next);
                      }}
                      type="checkbox"
                    />
                    <span>
                      <strong>{event.title}</strong>
                      <span className="block text-xs text-muted-foreground">
                        {new Date(event.start).toLocaleString()} ·{" "}
                        {event.sourceLocation}
                      </span>
                    </span>
                  </label>
                ))}
              </div>
              {preview ? (
                <p className="text-sm">
                  Review: {preview.diff.added} added · {preview.diff.changed}{" "}
                  changed · {preview.diff.removed} removed ·{" "}
                  {preview.diff.unchanged} unchanged
                </p>
              ) : null}
            </>
          ) : null}
          {error ? (
            <p className="text-sm text-destructive" role="alert">
              {error}
            </p>
          ) : null}
          <div className="flex justify-end gap-2">
            <button
              className="rounded border px-3 py-2 text-sm"
              onClick={() => onOpenChange(false)}
              type="button"
            >
              Cancel
            </button>
            <button
              className="rounded bg-primary px-3 py-2 text-sm text-primary-foreground disabled:opacity-50"
              disabled={!document || !proposal || busy}
              onClick={apply}
              type="button"
            >
              Apply approved changes
            </button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
