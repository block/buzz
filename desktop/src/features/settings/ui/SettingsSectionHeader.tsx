import type { ReactNode } from "react";

export function SettingsSectionHeader({
  action,
  description,
  title,
}: {
  action?: ReactNode;
  description: ReactNode;
  title: ReactNode;
}) {
  const copy = (
    <>
      <h2 className="text-2xl font-semibold tracking-tight">{title}</h2>
      <p className="text-base font-normal text-muted-foreground">
        {description}
      </p>
    </>
  );

  if (action) {
    return (
      <div className="mb-12 flex min-w-0 items-start justify-between gap-4">
        <div className="min-w-0 space-y-1">{copy}</div>
        <div className="shrink-0">{action}</div>
      </div>
    );
  }

  return <div className="mb-12 min-w-0 space-y-1">{copy}</div>;
}
