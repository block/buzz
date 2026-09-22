import { useTranslation } from "@/i18n";

import { hasMissingRequiredEnvKey } from "./personaRuntimeModel";

export function AdvancedRequiredBadge({
  envVars,
  requiredEnvKeys,
  show,
  testId,
}: {
  envVars?: Record<string, string>;
  requiredEnvKeys?: readonly string[];
  show?: boolean;
  testId: string;
}) {
  const { t } = useTranslation();
  const visible =
    show ?? hasMissingRequiredEnvKey(requiredEnvKeys ?? [], envVars ?? {});
  if (!visible) return null;
  return (
    <span
      aria-hidden="true"
      className="rounded-full bg-destructive/10 px-2 py-0.5 text-xs text-destructive"
      data-testid={testId}
    >
      {t("agents.advanced-badge.required")}
    </span>
  );
}
