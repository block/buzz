import type * as React from "react";
import type {
  AcpRuntimeCatalogEntry,
  CreatePersonaInput,
  UpdatePersonaInput,
} from "@/shared/api/types";

export type AgentDefinitionSubmitOptions = {
  publishCatalogUpdates: boolean;
};

export type AgentDefinitionDialogProps = {
  open: boolean;
  embedded?: boolean;
  title: string;
  description: string;
  submitLabel: string;
  initialValues: CreatePersonaInput | UpdatePersonaInput | null;
  error: Error | null;
  isPending: boolean;
  runtimes: AcpRuntimeCatalogEntry[];
  runtimeCatalogStatus?: "loading" | "ready" | "error";
  onDirtyChange?: (dirty: boolean) => void;
  onOpenChange: (open: boolean) => void;
  onSubmit: (
    input: CreatePersonaInput | UpdatePersonaInput,
    options: AgentDefinitionSubmitOptions,
  ) => Promise<unknown>;
  publishCatalogUpdatesOnSave?: boolean;
  createRunSection?: React.ReactNode;
  createSubmitBlocked?: boolean;
};
