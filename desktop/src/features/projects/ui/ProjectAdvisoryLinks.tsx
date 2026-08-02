import { OverviewRailSection } from "./ProjectOverviewPanel";

function shortLinkId(value: string) {
  return value.length <= 18 ? value : `${value.slice(0, 8)}…${value.slice(-6)}`;
}

function externalLinkDisplay(value: string) {
  const match = /^([a-z][a-z0-9+.-]{0,31}):(.*)$/i.exec(value);
  return {
    adapter: match?.[1].toLowerCase() ?? "external",
    id: shortLinkId(match?.[2] || value),
  };
}

export function ProjectAdvisoryLinks({
  primaryLabel,
  primaryId,
  externalId,
}: {
  primaryLabel: string;
  primaryId?: string | null;
  externalId?: string | null;
}) {
  const externalLink = externalId ? externalLinkDisplay(externalId) : null;
  if (!primaryId && !externalLink) return null;

  return (
    <OverviewRailSection title="Links">
      <dl className="space-y-1.5 text-xs text-muted-foreground">
        {primaryId ? (
          <div className="flex items-center justify-between gap-3">
            <dt>{primaryLabel}</dt>
            <dd title={primaryId}>
              <code className="text-foreground">#{shortLinkId(primaryId)}</code>
            </dd>
          </div>
        ) : null}
        {externalLink ? (
          <div className="flex items-center justify-between gap-3">
            <dt className="capitalize">{externalLink.adapter}</dt>
            <dd title={externalId ?? undefined}>
              <code className="text-foreground">{externalLink.id}</code>
            </dd>
          </div>
        ) : null}
      </dl>
      <p className="mt-1.5 text-2xs text-muted-foreground">
        Author-claimed metadata
      </p>
    </OverviewRailSection>
  );
}
