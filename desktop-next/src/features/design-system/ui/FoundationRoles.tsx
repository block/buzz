import type { CSSProperties } from "react";
import { FOUNDATION_GROUPS } from "@buzz/design-tokens/foundations";
/** Render geometry from the actual public custom properties, alongside registry values. */
export function FoundationRoles({
  name,
}: {
  name: (typeof FOUNDATION_GROUPS)[number]["name"];
}) {
  const group = FOUNDATION_GROUPS.find((group) => group.name === name);
  if (!group) throw new Error("Missing foundation group");
  return (
    <dl className="grid gap-6">
      {group.roles.map(([token, value, description]) => {
        const style: CSSProperties | undefined = token.startsWith("radius-")
          ? { width: "5rem", height: "5rem", borderRadius: `var(--${token})` }
          : token.startsWith("space-")
            ? { width: `var(--${token})`, height: "2rem" }
            : token.startsWith("control-")
              ? {
                  width: `var(--${token})`,
                  height: `var(--${token})`,
                  borderRadius: "var(--radius-control)",
                }
              : undefined;
        return (
          <div key={token} className="grid items-center gap-4 sm:grid-cols-3">
            <dt className="text-code font-mono">{token}</dt>
            <dd className="flex items-center gap-4">
              {style && (
                <span
                  aria-hidden="true"
                  className="bg-inverse shrink-0"
                  style={style}
                />
              )}
              <span className="text-body text-secondary">{value}</span>
            </dd>
            <dd className="text-caption text-secondary">{description}</dd>
          </div>
        );
      })}
    </dl>
  );
}
