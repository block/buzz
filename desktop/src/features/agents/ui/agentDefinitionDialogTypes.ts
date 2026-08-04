import type { ReactNode } from "react";

import type {
  AcpRuntimeCatalogEntry,
  AgentToolRequirement,
  CreatePersonaInput,
  UpdatePersonaInput,
} from "@/shared/api/types";

export type AgentDefinitionSubmitOptions = {
  publishCatalogUpdates: boolean;
};

export type AgentDefinitionDialogProps = {
  open: boolean;
  title: string;
  description: string;
  submitLabel: string;
  initialValues: CreatePersonaInput | UpdatePersonaInput | null;
  error: Error | null;
  isPending: boolean;
  runtimes: AcpRuntimeCatalogEntry[];
  runtimeCatalogStatus?: "loading" | "ready" | "error";
  onOpenChange: (open: boolean) => void;
  onSubmit: (
    input: CreatePersonaInput | UpdatePersonaInput,
    options: AgentDefinitionSubmitOptions,
  ) => Promise<unknown>;
  publishCatalogUpdatesOnSave?: boolean;
  createRunSection?:
    | ReactNode
    | ((toolRequirements: AgentToolRequirement[]) => ReactNode);
  createSubmitBlocked?:
    | boolean
    | ((toolRequirements: AgentToolRequirement[]) => boolean);
  createSubmitBlockReason?:
    | string
    | null
    | ((toolRequirements: AgentToolRequirement[]) => string | null);
};
