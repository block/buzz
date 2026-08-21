import { useQuery } from "@tanstack/react-query";
import { Check, CircleSlash2, Sparkles } from "lucide-react";

import {
  listProviderUsageCapabilities,
  type ProviderUsageCapability,
  type ProviderUsagePreference,
} from "@/shared/api/tauriProviderUsage";
import { cn } from "@/shared/lib/cn";
import {
  setProviderUsagePreference,
  useProviderUsagePreference,
} from "@/features/provider-usage/providerUsagePreference";

const FALLBACK_CAPABILITIES: ProviderUsageCapability[] = [
  {
    id: "codex",
    name: "Codex",
    availability: "temporarily_unavailable",
    detail: "Local capability check unavailable",
  },
  {
    id: "claude",
    name: "Claude",
    availability: "unsupported",
    detail: "No supported standalone personal allowance reader yet",
  },
  {
    id: "grok",
    name: "Grok",
    availability: "unsupported",
    detail: "Consumer allowance is available in Grok Settings",
  },
];

function ProviderChoice({
  capability,
  selected,
  onSelect,
}: {
  capability: ProviderUsageCapability;
  selected: boolean;
  onSelect: (preference: ProviderUsagePreference) => void;
}) {
  const disabled = capability.availability !== "available";
  return (
    <button
      aria-pressed={selected}
      className={cn(
        "flex min-h-20 items-start gap-3 rounded-lg border p-3 text-left transition-colors",
        selected
          ? "border-primary bg-primary/5"
          : "border-border/70 bg-background hover:bg-muted/50",
        disabled && "cursor-not-allowed opacity-65 hover:bg-background",
      )}
      disabled={disabled}
      onClick={() => onSelect(capability.id)}
      type="button"
    >
      <span
        className={cn(
          "mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded-full border",
          selected && "border-primary bg-primary text-primary-foreground",
        )}
      >
        {selected ? (
          <Check aria-hidden="true" className="h-3 w-3" />
        ) : disabled ? (
          <CircleSlash2 aria-hidden="true" className="h-3 w-3" />
        ) : null}
      </span>
      <span>
        <span className="block text-sm font-medium">{capability.name}</span>
        <span className="mt-1 block text-xs text-muted-foreground">
          {capability.detail}
        </span>
      </span>
    </button>
  );
}

export function ProviderUsageExperimentSettings({
  enabled,
}: {
  enabled: boolean;
}) {
  const preference = useProviderUsagePreference();
  const capabilitiesQuery = useQuery({
    queryKey: ["provider-usage-capabilities"],
    queryFn: listProviderUsageCapabilities,
    enabled,
    staleTime: Number.POSITIVE_INFINITY,
  });
  const capabilities = capabilitiesQuery.data ?? FALLBACK_CAPABILITIES;

  if (!enabled) return null;

  return (
    <div className="mt-3 border-t border-border/60 pt-3">
      <p className="mb-2 text-xs font-medium text-muted-foreground">
        Allowance provider
      </p>
      <div className="grid gap-2 sm:grid-cols-2">
        <button
          aria-pressed={preference === "auto"}
          className={cn(
            "flex min-h-20 items-start gap-3 rounded-lg border p-3 text-left transition-colors hover:bg-muted/50",
            preference === "auto"
              ? "border-primary bg-primary/5"
              : "border-border/70 bg-background",
          )}
          onClick={() => setProviderUsagePreference("auto")}
          type="button"
        >
          <span
            className={cn(
              "mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded-full border",
              preference === "auto" &&
                "border-primary bg-primary text-primary-foreground",
            )}
          >
            {preference === "auto" ? (
              <Check aria-hidden="true" className="h-3 w-3" />
            ) : (
              <Sparkles aria-hidden="true" className="h-3 w-3" />
            )}
          </span>
          <span>
            <span className="block text-sm font-medium">Auto</span>
            <span className="mt-1 block text-xs text-muted-foreground">
              Use the first supported local provider
            </span>
          </span>
        </button>
        {capabilities.map((capability) => (
          <ProviderChoice
            capability={capability}
            key={capability.id}
            onSelect={setProviderUsagePreference}
            selected={preference === capability.id}
          />
        ))}
      </div>
      <p className="mt-2 text-2xs text-muted-foreground">
        Personal allowance only. Buzz stores no provider credentials or raw
        usage responses and never publishes this data to Nostr.
      </p>
    </div>
  );
}
