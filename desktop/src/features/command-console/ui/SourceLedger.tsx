import type {
  SourceLedgerEntry,
  SourceFreshness,
} from "@/features/command-console/domain/briefContracts";
import { Badge } from "@/shared/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/shared/ui/card";

function formatTimestamp(value: string): string {
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(value));
}

export function SourceLedger({
  entries,
  freshness,
}: {
  entries: readonly SourceLedgerEntry[];
  freshness: SourceFreshness;
}) {
  const stale = new Set(freshness.staleSourceIds);
  return (
    <section aria-labelledby="command-brief-source-ledger-heading">
      <div className="mb-3">
        <h3
          className="text-base font-semibold"
          id="command-brief-source-ledger-heading"
        >
          Source ledger
        </h3>
        <p className="text-sm text-muted-foreground">
          Metadata only. Retrieved passages and hidden model context are not
          displayed.
        </p>
      </div>
      <div className="grid gap-3 sm:grid-cols-2">
        {entries.map((source) => (
          <Card
            id={`command-brief-source-${source.ledgerId}`}
            key={source.ledgerId}
            tabIndex={-1}
          >
            <CardHeader className="gap-2 pb-3">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <CardTitle className="break-all text-sm">
                  {source.ledgerId}
                </CardTitle>
                {stale.has(source.ledgerId) ? (
                  <Badge variant="warning">Stale source</Badge>
                ) : (
                  <Badge variant="success">Current source</Badge>
                )}
              </div>
            </CardHeader>
            <CardContent>
              <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-2 text-sm">
                <dt className="text-muted-foreground">Collection</dt>
                <dd className="break-all">{source.collection}</dd>
                <dt className="text-muted-foreground">Document</dt>
                <dd className="break-all">{source.documentId}</dd>
                <dt className="text-muted-foreground">Chunk</dt>
                <dd className="break-all">{source.chunkId}</dd>
                <dt className="text-muted-foreground">Location</dt>
                <dd>{source.quotedLocation.location}</dd>
                <dt className="text-muted-foreground">Retrieved</dt>
                <dd>
                  <time dateTime={source.retrievedAt}>
                    {formatTimestamp(source.retrievedAt)}
                  </time>
                </dd>
                <dt className="text-muted-foreground">Snapshot</dt>
                <dd className="break-all">{source.snapshotId}</dd>
              </dl>
            </CardContent>
          </Card>
        ))}
      </div>
    </section>
  );
}
