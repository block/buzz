import * as React from "react";

import type { ManagedAgent, UpdateManagedAgentInput } from "@/shared/api/types";

export type AgentLaunchFields = ReturnType<typeof useAgentLaunchFields>;

export function useAgentLaunchFields(agent: ManagedAgent, open: boolean) {
  const [commandWrapperCommand, setCommandWrapperCommand] = React.useState(
    agent.commandWrapper?.command ?? "",
  );
  const [commandWrapperArgs, setCommandWrapperArgs] = React.useState(
    agent.commandWrapper?.args.join(",") ?? "",
  );
  const [commandWrapperAuthorization, setCommandWrapperAuthorization] =
    React.useState(agent.commandWrapper?.authorization ?? null);
  const [workingDirectory, setWorkingDirectory] = React.useState(
    agent.workingDirectory ?? "",
  );
  const [saveBlocked, setSaveBlocked] = React.useState(false);

  // biome-ignore lint/correctness/useExhaustiveDependencies: polling must not wipe edits
  React.useEffect(() => {
    if (!open) return;
    setCommandWrapperCommand(agent.commandWrapper?.command ?? "");
    setCommandWrapperArgs(agent.commandWrapper?.args.join(",") ?? "");
    setCommandWrapperAuthorization(agent.commandWrapper?.authorization ?? null);
    setWorkingDirectory(agent.workingDirectory ?? "");
    setSaveBlocked(false);
  }, [open, agent.pubkey]);

  function buildUpdate(): Pick<
    UpdateManagedAgentInput,
    "commandWrapper" | "workingDirectory"
  > {
    const command = commandWrapperCommand.trim();
    const commandWrapper = command
      ? {
          command,
          args: commandWrapperArgs
            .split(",")
            .map((value) => value.trim())
            .filter(Boolean),
          authorization: commandWrapperAuthorization,
        }
      : null;
    const normalizedWorkingDirectory = workingDirectory.trim() || null;
    return {
      commandWrapper:
        JSON.stringify(commandWrapper) === JSON.stringify(agent.commandWrapper)
          ? undefined
          : commandWrapper,
      workingDirectory:
        normalizedWorkingDirectory === agent.workingDirectory
          ? undefined
          : normalizedWorkingDirectory,
    };
  }

  return {
    commandWrapperArgs,
    commandWrapperAuthorization,
    commandWrapperCommand,
    saveBlocked,
    workingDirectory,
    setCommandWrapperArgs,
    setCommandWrapperAuthorization,
    setCommandWrapperCommand,
    setSaveBlocked,
    setWorkingDirectory,
    buildUpdate,
  };
}
