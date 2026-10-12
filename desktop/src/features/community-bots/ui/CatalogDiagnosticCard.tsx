import * as React from "react";

import {
  type CatalogDiagnostic,
  type CatalogDiagnosticRole,
  isAdminRole,
  runCatalogDiagnostic,
} from "@/features/community-bots/lib/catalogDiagnostic";
import { Button } from "@/shared/ui/button";

function roleText(role: CatalogDiagnosticRole | null): string {
  if (role === null) return "unknown (membership list not readable)";
  if (role === "none") return "not in the membership list";
  return role;
}

function adminText(role: CatalogDiagnosticRole | null): string {
  if (role === null) return "unknown (membership list not readable)";
  return isAdminRole(role) ? `yes (${role})` : `no (${roleText(role)})`;
}

export function CatalogDiagnosticCard() {
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [result, setResult] = React.useState<CatalogDiagnostic | null>(null);

  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      setResult(await runCatalogDiagnostic());
    } catch (e) {
      setResult(null);
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      className="mt-6 rounded-xl border border-border/70 px-4 py-4 text-left text-sm"
      data-testid="bots-catalog-diagnostic"
    >
      <p className="font-medium text-foreground">
        Diagnostic (read-only): why is this empty?
      </p>
      <p className="mt-1 text-muted-foreground">
        Looks for the bot catalog on the relay using your current session. It
        publishes nothing.
      </p>
      <Button
        className="mt-3"
        data-testid="bots-catalog-diagnostic-run"
        disabled={busy}
        onClick={() => void run()}
        size="sm"
        type="button"
        variant="outline"
      >
        {busy ? "Checking…" : "Check catalog on relay"}
      </Button>
      {error ? (
        <p
          className="mt-3 text-destructive"
          data-testid="bots-catalog-diagnostic-error"
        >
          Check failed: {error}
        </p>
      ) : null}
      {result ? (
        <div
          className="mt-3 space-y-1 break-all text-muted-foreground"
          data-testid="bots-catalog-diagnostic-result"
        >
          <p data-testid="bots-catalog-diagnostic-exists">
            Catalog event on relay:{" "}
            {result.event ? "yes" : "no (nothing published yet)"}
          </p>
          {result.event ? (
            <>
              <p>Event id: {result.event.id}</p>
              <p>
                Created:{" "}
                {new Date(result.event.createdAt * 1000).toLocaleString()} (
                {result.event.createdAt})
              </p>
              <p>Author: {result.event.authorNpub}</p>
              <p data-testid="bots-catalog-diagnostic-botcount">
                Bots in catalog: {result.event.botCount}
                {result.event.rawBotEntries !== null &&
                result.event.rawBotEntries !== result.event.botCount
                  ? ` (${result.event.rawBotEntries} entries, some invalid and skipped)`
                  : ""}
              </p>
              <p data-testid="bots-catalog-diagnostic-author-role">
                Author is owner/admin: {adminText(result.authorRole)}
              </p>
            </>
          ) : null}
          <p>Signed in as: {result.myNpub}</p>
          <p data-testid="bots-catalog-diagnostic-my-role">
            You are an admin: {adminText(result.myRole)}
          </p>
          <p
            className="pt-2 text-foreground"
            data-testid="bots-catalog-diagnostic-hint"
          >
            {result.hint}
          </p>
        </div>
      ) : null}
    </div>
  );
}
