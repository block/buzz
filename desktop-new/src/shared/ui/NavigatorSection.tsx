import type { ReactNode } from "react";

export function NavigatorSection({
  label,
  children,
}: {
  label: string;
  children: ReactNode;
}) {
  return (
    <section className="navigator-section">
      <h2 className="navigator-section-label text-body text-secondary">
        {label}
      </h2>
      <div className="navigator-section-rows">{children}</div>
    </section>
  );
}
