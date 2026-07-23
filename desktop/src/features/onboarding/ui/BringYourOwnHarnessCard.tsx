import { Card } from "@/shared/ui/card";
import { cn } from "@/shared/lib/cn";
import { CustomHarnessFields } from "@/features/agents/ui/CustomHarnessFields";

export function BringYourOwnHarnessCard({
  args,
  command,
  onArgsChange,
  onCommandChange,
  onSelectedChange,
  selected,
}: {
  args: string;
  command: string;
  onArgsChange: (value: string) => void;
  onCommandChange: (value: string) => void;
  onSelectedChange: (selected: boolean) => void;
  selected: boolean;
}) {
  const ready = selected && command.trim().length > 0;

  return (
    <Card
      className={cn(
        "w-full max-w-[592px] select-none items-stretch px-5 py-5 text-left",
        ready && "ring-1 ring-[var(--buzz-welcome-chartreuse)]/50",
      )}
      data-ready={ready ? "true" : "false"}
      data-testid="onboarding-runtime-custom"
      variant="textured"
    >
      <label className="flex cursor-pointer items-start gap-3">
        <input
          checked={selected}
          className="mt-1"
          data-testid="onboarding-runtime-custom-toggle"
          onChange={(event) => onSelectedChange(event.target.checked)}
          type="checkbox"
        />
        <span className="min-w-0 flex-1 space-y-1">
          <span className="block text-sm font-medium text-foreground">
            Bring your own harness
          </span>
          <span className="block text-xs leading-5 text-muted-foreground">
            Any binary that speaks{" "}
            <a
              className="underline underline-offset-2"
              href="https://agentclientprotocol.com/"
              rel="noreferrer"
              target="_blank"
            >
              ACP
            </a>{" "}
            over stdio — for example Cursor{" "}
            <code className="font-mono text-2xs">agent acp</code> or your own
            adapter. Buzz does not verify PATH until the agent starts.
          </span>
        </span>
      </label>

      {selected ? (
        <div className="mt-4">
          <CustomHarnessFields
            args={args}
            argsId="onboarding-custom-args"
            argsTestId="onboarding-custom-args"
            command={command}
            commandId="onboarding-custom-command"
            commandTestId="onboarding-custom-command"
            onArgsChange={onArgsChange}
            onCommandChange={onCommandChange}
            size="onboarding"
          />
        </div>
      ) : null}
    </Card>
  );
}
