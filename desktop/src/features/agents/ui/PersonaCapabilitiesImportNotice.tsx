import type { PersonaCapabilities } from "@/shared/api/personaTypes";

/** Executable settings are shown before the existing import confirmation. */
export function PersonaCapabilitiesImportNotice({
  value,
}: {
  value?: PersonaCapabilities;
}) {
  if (!value || Object.keys(value).length === 0) return null;
  // Always visible: executable configuration is never hidden behind a summary.
  return (
    <section
      className="space-y-2 rounded-md border border-border p-3 text-sm"
      data-testid="persona-capabilities-import-notice"
    >
      <p className="font-medium">Assigned skills, MCP commands and scripts</p>
      <p className="text-xs text-muted-foreground">
        These settings are imported with the persona. MCP commands, skill
        scripts and lifecycle scripts may execute with your access when the
        agent runs. Referenced files are not bundled. Review paths and commands
        before launching.
      </p>
      <pre className="max-h-48 overflow-auto whitespace-pre-wrap break-words text-xs">
        {JSON.stringify(value, null, 2)}
      </pre>
    </section>
  );
}
