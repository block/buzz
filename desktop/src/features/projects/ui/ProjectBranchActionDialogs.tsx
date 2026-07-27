import type { useProjectBranchActions } from "@/features/projects/branchMutations";
import {
  CreateProjectBranchDialog,
  DeleteProjectBranchDialog,
} from "./ProjectBranchDialogs";

export function ProjectBranchActionDialogs({
  actions,
  activeBranch,
  activeBranchCommit,
  existingBranches,
}: {
  actions: ReturnType<typeof useProjectBranchActions>;
  activeBranch: string | null;
  activeBranchCommit: string | null;
  existingBranches: string[];
}) {
  return (
    <>
      <CreateProjectBranchDialog
        existingBranches={existingBranches}
        onCreate={actions.handleCreate}
        onOpenChange={actions.setCreateOpen}
        open={actions.createOpen}
        pending={actions.createPending}
        sourceBranch={activeBranch ?? ""}
        sourceCommit={activeBranchCommit}
      />
      <DeleteProjectBranchDialog
        branch={activeBranch ?? ""}
        onDelete={actions.handleDelete}
        onOpenChange={actions.setDeleteOpen}
        open={actions.deleteOpen}
        pending={actions.deletePending}
      />
    </>
  );
}
