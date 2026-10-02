import {
  CreateProjectWorkItemDialog,
  type CreateProjectWorkItemDialogInput,
} from "./CreateProjectWorkItemDialog";

export type CreateIssueDialogInput = CreateProjectWorkItemDialogInput;

export function CreateIssueDialog({
  isCreating,
  onCreate,
  onOpenChange,
  open,
  projectName,
}: {
  isCreating: boolean;
  onCreate: (input: CreateIssueDialogInput) => Promise<void>;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  projectName: string;
}) {
  return (
    <CreateProjectWorkItemDialog
      bodyPlaceholder="Add context, expected behavior, or reproduction steps"
      description={`Create a task in ${projectName}`}
      isCreating={isCreating}
      itemName="issue"
      onCreate={onCreate}
      onOpenChange={onOpenChange}
      open={open}
      title="Create a task"
      titlePlaceholder="Describe the task"
    />
  );
}
