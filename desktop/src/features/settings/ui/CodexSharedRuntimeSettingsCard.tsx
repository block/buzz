import { CodexSharedRuntimePanel } from "@/features/agents/ui/CodexSharedRuntimePanel";
import { SectionHeader } from "@/shared/ui/PageHeader";

export function CodexSharedRuntimeSettingsCard() {
  return (
    <section className="min-w-0 space-y-4" data-testid="settings-codex-runtime">
      <SectionHeader
        title="Codex shared runtime"
        description="The computer-level connection used by Codex Desktop and every Codex task agent."
      />
      <CodexSharedRuntimePanel />
    </section>
  );
}
