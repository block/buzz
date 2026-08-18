import * as React from "react";
import { Plus, X } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Switch } from "@/shared/ui/switch";
import {
  getAgentRoutingPolicy,
  setAgentRoutingPolicy,
  type RoutingMatchKind,
  type RoutingPolicy,
} from "@/shared/api/routingPolicy";
import {
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
  PERSONA_LABEL_OPTIONAL_CLASS,
} from "./agentConfigOptions";

/** Env var the harness reads the policy path from (`buzz-acp` routing.rs). */
export const ROUTING_POLICY_ENV_KEY = "BUZZ_ROUTING_POLICY";

/** A rule row. `id` is local only — it keeps React keys stable while typing. */
type RuleRow = {
  id: string;
  name: string;
  matchKind: RoutingMatchKind;
  /** Comma-separated in the UI; split on save. */
  phrases: string;
  model: string;
};

function newRuleId(): string {
  return crypto.randomUUID();
}

function toRows(policy: RoutingPolicy | null): RuleRow[] {
  return (policy?.rules ?? []).map((rule) => ({
    id: newRuleId(),
    name: rule.name ?? "",
    matchKind: rule.matchKind,
    phrases: rule.any.join(", "),
    model: rule.model,
  }));
}

function splitPhrases(raw: string): string[] {
  return raw
    .split(",")
    .map((phrase) => phrase.trim())
    .filter((phrase) => phrase.length > 0);
}

/**
 * Per-turn model routing for one agent.
 *
 * Writes the JSON policy file that `buzz-acp` reads, then points the agent's
 * `BUZZ_ROUTING_POLICY` env var at it via `onEnvVarChange`. The env change is
 * staged in the dialog's env map and lands with the dialog's own save — the
 * file write happens immediately, because the file is not part of the agent
 * record and has nothing to wait for.
 *
 * Routing is opt-in and fails open on the harness side: an unreadable or
 * disabled policy means turns run on the agent's configured model, exactly as
 * they did before. The UI mirrors that — turning routing off deletes the file
 * and drops the env var rather than leaving a dormant one behind.
 */
export function RoutingPolicyEditor({
  disabled,
  envValue,
  pubkey,
  onEnvVarChange,
}: {
  disabled: boolean;
  /** Current `BUZZ_ROUTING_POLICY` value in the dialog's env map, if any. */
  envValue: string | undefined;
  pubkey: string;
  onEnvVarChange: (key: string, value: string) => void;
}) {
  const [loaded, setLoaded] = React.useState(false);
  const [enabled, setEnabled] = React.useState(false);
  const [rows, setRows] = React.useState<RuleRow[]>([]);
  const [defaultModel, setDefaultModel] = React.useState("");
  const [path, setPath] = React.useState<string | null>(null);
  const [classifier, setClassifier] = React.useState<unknown>(undefined);
  const [saving, setSaving] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [savedAt, setSavedAt] = React.useState<number | null>(null);

  React.useEffect(() => {
    let cancelled = false;
    void getAgentRoutingPolicy(pubkey)
      .then((file) => {
        if (cancelled) return;
        setPath(file.path);
        setEnabled(file.policy?.enabled ?? false);
        setRows(toRows(file.policy));
        setDefaultModel(file.policy?.defaultModel ?? "");
        setClassifier(file.policy?.classifier);
        setLoaded(true);
      })
      .catch((loadError: unknown) => {
        if (cancelled) return;
        setError(
          loadError instanceof Error ? loadError.message : String(loadError),
        );
        setLoaded(true);
      });
    return () => {
      cancelled = true;
    };
  }, [pubkey]);

  const updateRow = (id: string, patch: Partial<RuleRow>) => {
    setRows((current) =>
      current.map((row) => (row.id === id ? { ...row, ...patch } : row)),
    );
    setSavedAt(null);
  };

  const handleSave = async () => {
    setSaving(true);
    setError(null);
    try {
      if (!enabled && rows.length === 0) {
        // Nothing to route with. Delete the file and drop the env var so the
        // agent is left in exactly the state it had before routing was touched.
        const file = await setAgentRoutingPolicy(pubkey, null);
        setPath(file.path);
        onEnvVarChange(ROUTING_POLICY_ENV_KEY, "");
        setSavedAt(Date.now());
        return;
      }

      const policy: RoutingPolicy = {
        enabled,
        rules: rows.map((row) => ({
          name: row.name.trim() ? row.name.trim() : null,
          matchKind: row.matchKind,
          any: splitPhrases(row.phrases),
          model: row.model,
        })),
        defaultModel: defaultModel.trim() ? defaultModel.trim() : null,
        classifier,
      };
      const file = await setAgentRoutingPolicy(pubkey, policy);
      setPath(file.path);
      onEnvVarChange(ROUTING_POLICY_ENV_KEY, file.path);
      setSavedAt(Date.now());
    } catch (saveError: unknown) {
      setError(
        saveError instanceof Error ? saveError.message : String(saveError),
      );
    } finally {
      setSaving(false);
    }
  };

  const envPointsAtPolicy = !!envValue && !!path && envValue === path;

  return (
    <div className="space-y-3" data-testid="routing-policy-editor">
      <div className="flex items-start justify-between gap-3">
        <div>
          <p className="text-sm font-medium">
            Model routing
            <span className={PERSONA_LABEL_OPTIONAL_CLASS}>optional</span>
          </p>
          <p className="text-xs text-muted-foreground">
            Send a turn to a different model based on what it says. Rules are
            checked in order; the first match wins.
          </p>
        </div>
        <Switch
          aria-label="Enable model routing"
          checked={enabled}
          data-testid="routing-policy-enabled"
          disabled={disabled || !loaded}
          onCheckedChange={(next) => {
            setEnabled(next);
            setSavedAt(null);
          }}
        />
      </div>

      {!loaded ? (
        <p className="text-xs text-muted-foreground">Loading routing policy…</p>
      ) : (
        <>
          <div className="space-y-2">
            {rows.length === 0 ? (
              <p className="text-xs text-muted-foreground">
                No rules yet. Without rules, every turn falls back to the
                default model below — or to the agent's own model if that is
                blank too.
              </p>
            ) : null}

            {rows.map((row, index) => (
              <div className="flex items-center gap-2" key={row.id}>
                <div
                  className={cn(
                    "flex min-h-11 flex-1 items-center px-3",
                    PERSONA_FIELD_SHELL_CLASS,
                  )}
                >
                  <Input
                    aria-label={`Rule ${index + 1} name`}
                    className={cn("h-8 px-0 py-0", PERSONA_FIELD_CONTROL_CLASS)}
                    data-testid="routing-rule-name"
                    disabled={disabled}
                    onChange={(event) =>
                      updateRow(row.id, { name: event.target.value })
                    }
                    placeholder="name"
                    value={row.name}
                  />
                </div>
                <select
                  aria-label={`Rule ${index + 1} match kind`}
                  className="h-9 rounded-md border border-input bg-background px-2 text-xs"
                  data-testid="routing-rule-match-kind"
                  disabled={disabled}
                  onChange={(event) =>
                    updateRow(row.id, {
                      matchKind: event.target.value as RoutingMatchKind,
                    })
                  }
                  value={row.matchKind}
                >
                  <option value="contains">any of</option>
                  <option value="contains_all">all of</option>
                </select>
                <div
                  className={cn(
                    "flex min-h-11 flex-[2] items-center px-3",
                    PERSONA_FIELD_SHELL_CLASS,
                  )}
                >
                  <Input
                    aria-label={`Rule ${index + 1} phrases`}
                    className={cn("h-8 px-0 py-0", PERSONA_FIELD_CONTROL_CLASS)}
                    data-testid="routing-rule-phrases"
                    disabled={disabled}
                    onChange={(event) =>
                      updateRow(row.id, { phrases: event.target.value })
                    }
                    placeholder="migration, schema"
                    value={row.phrases}
                  />
                </div>
                <div
                  className={cn(
                    "flex min-h-11 flex-[2] items-center px-3",
                    PERSONA_FIELD_SHELL_CLASS,
                  )}
                >
                  <Input
                    aria-label={`Rule ${index + 1} model`}
                    className={cn(
                      "h-8 px-0 py-0 font-mono",
                      PERSONA_FIELD_CONTROL_CLASS,
                    )}
                    data-testid="routing-rule-model"
                    disabled={disabled}
                    onChange={(event) =>
                      updateRow(row.id, { model: event.target.value })
                    }
                    placeholder="model id"
                    value={row.model}
                  />
                </div>
                <Button
                  aria-label={`Remove rule ${index + 1}`}
                  data-testid="routing-rule-remove"
                  disabled={disabled}
                  onClick={() => {
                    setRows((current) =>
                      current.filter((candidate) => candidate.id !== row.id),
                    );
                    setSavedAt(null);
                  }}
                  size="icon"
                  type="button"
                  variant="ghost"
                >
                  <X className="h-4 w-4" />
                </Button>
              </div>
            ))}

            <Button
              data-testid="routing-rule-add"
              disabled={disabled}
              onClick={() => {
                setRows((current) => [
                  ...current,
                  {
                    id: newRuleId(),
                    name: "",
                    matchKind: "contains",
                    phrases: "",
                    model: "",
                  },
                ]);
                setSavedAt(null);
              }}
              size="sm"
              type="button"
              variant="outline"
            >
              <Plus className="mr-1 h-4 w-4" />
              Add rule
            </Button>
          </div>

          <div className="space-y-1">
            <label
              className="text-xs text-muted-foreground"
              htmlFor="routing-default-model"
            >
              Default model
              <span className={PERSONA_LABEL_OPTIONAL_CLASS}>
                used when no rule matches
              </span>
            </label>
            <div
              className={cn(
                "flex min-h-11 items-center px-3",
                PERSONA_FIELD_SHELL_CLASS,
              )}
            >
              <Input
                className={cn(
                  "h-8 px-0 py-0 font-mono",
                  PERSONA_FIELD_CONTROL_CLASS,
                )}
                data-testid="routing-default-model"
                disabled={disabled}
                id="routing-default-model"
                onChange={(event) => {
                  setDefaultModel(event.target.value);
                  setSavedAt(null);
                }}
                placeholder="Leave blank to use the agent's own model"
                value={defaultModel}
              />
            </div>
          </div>

          {classifier ? (
            <p className="text-xs text-muted-foreground">
              This policy also has a local classifier configured in the file.
              Buzz keeps it as-is — edit it in{" "}
              <span className="font-mono break-all">{path}</span>.
            </p>
          ) : null}

          <div className="flex items-center gap-2">
            <Button
              data-testid="routing-policy-save"
              disabled={disabled || saving}
              onClick={() => void handleSave()}
              size="sm"
              type="button"
              variant="outline"
            >
              {saving ? "Saving…" : "Save routing policy"}
            </Button>
            {savedAt !== null && !error ? (
              <p className="text-xs text-muted-foreground">
                Saved. Save the agent to apply — the harness reads the policy at
                start-up.
              </p>
            ) : null}
          </div>

          {error ? (
            <p
              className="text-xs text-destructive"
              data-testid="routing-policy-error"
            >
              {error}
            </p>
          ) : null}

          {enabled && !envPointsAtPolicy ? (
            <p className="text-xs text-muted-foreground">
              Routing is not active yet: {ROUTING_POLICY_ENV_KEY} does not point
              at this policy. Save the routing policy to set it.
            </p>
          ) : null}
        </>
      )}
    </div>
  );
}
