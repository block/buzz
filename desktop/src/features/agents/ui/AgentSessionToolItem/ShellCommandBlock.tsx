import { Terminal } from "lucide-react";

import { parseShellToolOutput } from "../agentSessionUtils";

export function ShellCommandBlock({
  command,
  result,
}: {
  command: string;
  result: string;
}) {
  const output = parseShellToolOutput(result);
  const stdout = output.stdout.trimEnd();

  return (
    <div
      className="rounded-lg bg-muted/40 px-3 py-2 font-mono text-xs leading-5"
      data-testid="transcript-shell-command"
    >
      <p className="whitespace-pre-wrap wrap-break-word text-muted-foreground/70">
        <Terminal className="mr-2 inline h-3.5 w-3.5 align-[-0.1875rem] text-accent-foreground" />
        {command}
      </p>
      {stdout ? (
        <pre className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap wrap-break-word text-foreground">
          {stdout}
        </pre>
      ) : null}
    </div>
  );
}
