import * as React from "react";

type ProjectBranch = { name: string; commit: string };

export function useOptimisticProjectBranches(input: {
  defaultBranch: string | null;
  observedBranches: ProjectBranch[];
  projectId: string;
  referencedBranches: Array<string | null>;
}) {
  const [state, setState] = React.useState<{
    projectId: string;
    branches: ProjectBranch[];
  }>({ projectId: input.projectId, branches: [] });
  const optimisticBranches =
    state.projectId === input.projectId ? state.branches : [];

  const branchOptions = React.useMemo(() => {
    const names = [
      input.defaultBranch,
      ...input.observedBranches.map((branch) => branch.name),
      ...optimisticBranches.map((branch) => branch.name),
      ...input.referencedBranches,
    ].filter((name): name is string => Boolean(name));
    return [...new Set(names)];
  }, [
    input.defaultBranch,
    input.observedBranches,
    input.referencedBranches,
    optimisticBranches,
  ]);

  const managedBranches = React.useMemo(() => {
    const branches = [...input.observedBranches];
    for (const branch of optimisticBranches) {
      if (!branches.some((candidate) => candidate.name === branch.name)) {
        branches.push(branch);
      }
    }
    return branches;
  }, [input.observedBranches, optimisticBranches]);

  React.useEffect(() => {
    const observedNames = new Set(
      input.observedBranches.map((branch) => branch.name),
    );
    if (observedNames.size === 0) return;
    setState((current) => {
      if (current.projectId !== input.projectId) return current;
      const pending = current.branches.filter(
        (branch) => !observedNames.has(branch.name),
      );
      return pending.length === current.branches.length
        ? current
        : { ...current, branches: pending };
    });
  }, [input.observedBranches, input.projectId]);

  const rememberBranch = React.useCallback(
    (branch: ProjectBranch) => {
      setState((current) => {
        const branches =
          current.projectId === input.projectId ? current.branches : [];
        return branches.some((candidate) => candidate.name === branch.name)
          ? current
          : { projectId: input.projectId, branches: [...branches, branch] };
      });
    },
    [input.projectId],
  );

  const forgetBranch = React.useCallback(
    (branchName: string) => {
      setState((current) => {
        if (current.projectId !== input.projectId) return current;
        return {
          ...current,
          branches: current.branches.filter(
            (branch) => branch.name !== branchName,
          ),
        };
      });
    },
    [input.projectId],
  );

  return { branchOptions, forgetBranch, managedBranches, rememberBranch };
}
