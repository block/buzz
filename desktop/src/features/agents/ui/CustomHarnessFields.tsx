import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import {
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
  PERSONA_LABEL_OPTIONAL_CLASS,
} from "@/features/agents/ui/agentConfigOptions";

type CustomHarnessFieldsProps = {
  args: string;
  command: string;
  commandTestId?: string;
  argsTestId?: string;
  commandId: string;
  argsId: string;
  disabled?: boolean;
  /** Larger onboarding-style controls vs compact settings controls. */
  size?: "default" | "onboarding";
  onArgsChange: (value: string) => void;
  onCommandChange: (value: string) => void;
};

export function CustomHarnessFields({
  args,
  command,
  commandTestId,
  argsTestId,
  commandId,
  argsId,
  disabled,
  size = "default",
  onArgsChange,
  onCommandChange,
}: CustomHarnessFieldsProps) {
  const onboarding = size === "onboarding";

  return (
    <div className={cn("space-y-4", onboarding && "grid gap-3 sm:grid-cols-2 sm:space-y-0")}>
      <div className="space-y-1.5">
        <label
          className={cn(
            onboarding ? "text-xs font-medium text-foreground" : "text-sm font-medium text-foreground",
            !onboarding && "pl-0",
          )}
          htmlFor={commandId}
        >
          Agent command
        </label>
        {onboarding ? (
          <Input
            autoCorrect="off"
            className="h-10 rounded-xl border-foreground/15 bg-white"
            data-testid={commandTestId}
            disabled={disabled}
            id={commandId}
            onChange={(event) => onCommandChange(event.target.value)}
            placeholder="agent"
            spellCheck={false}
            value={command}
          />
        ) : (
          <div
            className={cn(
              "flex min-h-11 items-center px-3",
              PERSONA_FIELD_SHELL_CLASS,
            )}
          >
            <Input
              autoCorrect="off"
              className={cn(
                "h-8 px-0 py-0 leading-6",
                PERSONA_FIELD_CONTROL_CLASS,
              )}
              data-testid={commandTestId}
              disabled={disabled}
              id={commandId}
              onChange={(event) => onCommandChange(event.target.value)}
              placeholder="Full path or command on PATH (e.g. agent)"
              spellCheck={false}
              value={command}
            />
          </div>
        )}
      </div>
      <div className="space-y-1.5">
        <label
          className={cn(
            onboarding ? "text-xs font-medium text-foreground" : "text-sm font-medium text-foreground",
          )}
          htmlFor={argsId}
        >
          Args
          <span
            className={
              onboarding
                ? "ml-1 font-normal text-muted-foreground"
                : PERSONA_LABEL_OPTIONAL_CLASS
            }
          >
            Optional
          </span>
        </label>
        {onboarding ? (
          <Input
            autoCorrect="off"
            className="h-10 rounded-xl border-foreground/15 bg-white"
            data-testid={argsTestId}
            disabled={disabled}
            id={argsId}
            onChange={(event) => onArgsChange(event.target.value)}
            placeholder="acp"
            spellCheck={false}
            value={args}
          />
        ) : (
          <div
            className={cn(
              "flex min-h-11 items-center px-3",
              PERSONA_FIELD_SHELL_CLASS,
            )}
          >
            <Input
              autoCorrect="off"
              className={cn(
                "h-8 px-0 py-0 leading-6",
                PERSONA_FIELD_CONTROL_CLASS,
              )}
              data-testid={argsTestId}
              disabled={disabled}
              id={argsId}
              onChange={(event) => onArgsChange(event.target.value)}
              placeholder="Comma-separated (e.g. acp)"
              spellCheck={false}
              value={args}
            />
          </div>
        )}
        {!onboarding ? (
          <p className="text-xs text-muted-foreground">
            Must speak ACP over stdio. Buzz resolves the command on PATH when
            the agent starts.
          </p>
        ) : null}
      </div>
    </div>
  );
}
