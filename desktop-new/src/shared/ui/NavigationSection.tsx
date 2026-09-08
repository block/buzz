import type { ReactNode } from "react";

export function NavigationSection({
  label,
  action,
  children,
}: {
  label: string;
  action?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="navigation-section">
      <div className="navigation-section-heading">
        <h2 className="navigation-section-label text-body text-secondary">
          {label}
        </h2>
        {action}
      </div>
      <div className="navigation-section-rows">{children}</div>
    </section>
  );
}
