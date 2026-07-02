import { Terminal } from "lucide-react";

import { ScrollFadeMonoPanel } from "../FileContentBlock";
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
      className="rounded-lg bg-muted/40 px-3 font-mono text-xs leading-5"
      data-testid="transcript-shell-command"
    >
      <ScrollFadeMonoPanel fadeFromClassName="from-muted/40">
        <p className="whitespace-pre-wrap wrap-break-word text-muted-foreground/70">
          <Terminal className="mr-2 inline h-3.5 w-3.5 align-[-0.1875rem] text-accent" />
          {command}
        </p>
      </ScrollFadeMonoPanel>
      {stdout ? (
        <pre className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap wrap-break-word py-2 text-foreground">
          {stdout}
        </pre>
      ) : null}
    </div>
  );
}
