// Consistent small placeholder for desktop-only features in the web build.
import type { ReactNode } from "react";
import { CAPABILITY_GROUPS, webCapability } from "./capabilities";

export function NativeOnly({
  group,
  children,
}: {
  group: string;
  children?: ReactNode;
}) {
  const cap = webCapability(group);
  if (cap.hidden) return null;
  const label = CAPABILITY_GROUPS.find((g) => g.id === group)?.label ?? group;
  return (
    <div
      role="note"
      data-testid="native-only"
      data-capability={group}
      style={{
        padding: "8px 12px",
        border: "1px dashed currentColor",
        borderRadius: 8,
        opacity: 0.7,
        fontSize: 13,
      }}
    >
      Available in the desktop app{children ? <> — {children}</> : null}
      <span style={{ display: "block", fontSize: 11, opacity: 0.7 }}>
        {label}
      </span>
    </div>
  );
}
