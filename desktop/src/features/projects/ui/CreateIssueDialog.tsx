import { useTranslation } from "@/i18n";

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
  const { t } = useTranslation();
  return (
    <CreateProjectWorkItemDialog
      bodyPlaceholder="Add context, expected behavior, or reproduction steps"
      description={t("projects.create-task-dialog.description-in", {
        name: projectName,
      })}
      isCreating={isCreating}
      itemName="issue"
      onCreate={onCreate}
      onOpenChange={onOpenChange}
      open={open}
      title={t("projects.create-task-dialog.title")}
      titlePlaceholder="Describe the task"
    />
  );
}
