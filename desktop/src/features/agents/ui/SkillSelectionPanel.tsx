import { useQuery } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { Layers, Loader2 } from "lucide-react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { Switch } from "@/shared/ui/switch";

/**
 * A single installed skill discovered across the runtime skill roots.
 * Returned by the `list_installed_skills` Tauri command.
 */
export type InstalledSkill = {
  name: string;
  description: string;
  path: string;
  source: string;
};

/** Env var (per-agent) that carries the comma-separated skill allowlist. */
export const BUZZ_AGENT_SKILLS_ENV = "BUZZ_AGENT_SKILLS";

/** Serialize enabled skill names into the `BUZZ_AGENT_SKILLS` env value. */
export function skillNamesToEnvValue(names: readonly string[]): string {
  return names.join(",");
}

/** Parse the `BUZZ_AGENT_SKILLS` env value back into enabled skill names. */
export function envValueToSkillNames(value: string | undefined): string[] {
  return (value ?? "")
    .split(/[\s,]+/)
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}

type SkillSelectionPanelProps = {
  /** Currently enabled skill names (drives the toggles). */
  value: readonly string[];
  /** Called with the new full set of enabled skill names on any toggle. */
  onChange: (names: string[]) => void;
  className?: string;
};

/**
 * "Skill selection" — the fixed-height, inner-scrollable list of every
 * installed skill with an on/off toggle per skill.
 *
 * Everyone starts with everything off; only enabled skills are written to the
 * agent's `BUZZ_AGENT_SKILLS` allowlist and offered via `load_skill`. The outer
 * box keeps a fixed height (`h-64`) and the list scrolls inside it, so the box
 * never moves with the content.
 */
export function SkillSelectionPanel({
  value,
  onChange,
  className,
}: SkillSelectionPanelProps) {
  const enabled = React.useMemo(() => new Set(value), [value]);
  const query = useQuery({
    queryKey: ["list_installed_skills"],
    queryFn: () => invoke<InstalledSkill[]>("list_installed_skills"),
  });

  const skills = query.data ?? [];

  const toggle = (name: string, on: boolean) => {
    const next = new Set(value);
    if (on) {
      next.add(name);
    } else {
      next.delete(name);
    }
    onChange(Array.from(next).sort());
  };

  return (
    <div className={cn("flex flex-col gap-2", className)}>
      <div className="flex items-center justify-between">
        <span className="flex items-center gap-1.5 text-sm font-medium">
          <Layers className="h-3.5 w-3.5" />
          Skill selection
        </span>
        <span className="text-xs text-muted-foreground">
          {enabled.size} / {skills.length} enabled
        </span>
      </div>

      <div className="h-64 overflow-hidden rounded-xl border border-input bg-muted/40">
        {query.isLoading && (
          <div className="flex h-full items-center justify-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            Loading skills…
          </div>
        )}
        {!query.isLoading && skills.length === 0 && (
          <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
            {query.isError ? "Couldn't load skills." : "No skills installed."}
          </div>
        )}
        {!query.isLoading && skills.length > 0 && (
          <div className="h-full overflow-y-auto p-1.5">
            {skills.map((skill) => {
              const active = enabled.has(skill.name);
              return (
                <label
                  key={skill.name}
                  className="flex cursor-pointer items-center justify-between gap-3 rounded-lg px-2 py-2 hover:bg-muted/60"
                >
                  <span className="min-w-0">
                    <span className="block truncate text-sm font-medium">
                      {skill.name}
                    </span>
                    {skill.description && (
                      <span className="block truncate text-xs text-muted-foreground">
                        {skill.description}
                      </span>
                    )}
                  </span>
                  <Switch
                    checked={active}
                    onCheckedChange={(on) => toggle(skill.name, on)}
                    aria-label={`Enable ${skill.name}`}
                  />
                </label>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
