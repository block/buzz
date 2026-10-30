// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT } from "@/shared/i18n/uiTranslation";
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
      bodyPlaceholder={uiT(
        "Add context, expected behavior, or reproduction steps",
      )}
      description={uiT("Create a task in {0}", { p0: projectName })}
      isCreating={isCreating}
      itemName="issue"
      onCreate={onCreate}
      onOpenChange={onOpenChange}
      open={open}
      title={uiT("Create a task")}
      titlePlaceholder={uiT("Describe the task")}
    />
  );
}
