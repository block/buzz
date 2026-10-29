import { useMutation } from "@tanstack/react-query";

import { resolveSnapshotAvatarPng } from "@/features/agents/ui/snapshotAvatarPng";
import {
  confirmAgentSnapshotImport,
  encodeAgentSnapshotForSend,
  exportAgentSnapshot,
  previewAgentSnapshotImport,
  type AgentSnapshotImportConfirm,
  type AgentSnapshotImportPreview,
  type AgentSnapshotImportResult,
  type EncodedSnapshotPayload,
  type SnapshotFormat,
  type SnapshotMemoryLevel,
} from "@/shared/api/tauriPersonas";

export function useExportAgentSnapshotMutation() {
  return useMutation({
    mutationFn: async ({
      id,
      memoryLevel,
      format,
      memorySourcePubkey,
      avatarUrl,
    }: {
      id: string;
      memoryLevel: SnapshotMemoryLevel;
      format: SnapshotFormat;
      memorySourcePubkey?: string | null;
      avatarUrl?: string | null;
    }) => {
      const avatarPngDataUrl =
        format === "png"
          ? await resolveSnapshotAvatarPng(avatarUrl)
          : undefined;
      return exportAgentSnapshot(
        id,
        memoryLevel,
        format,
        memorySourcePubkey,
        avatarPngDataUrl,
      );
    },
  });
}

export function useEncodeAgentSnapshotForSendMutation() {
  return useMutation({
    mutationFn: ({
      id,
      memoryLevel,
      format,
      memorySourcePubkey,
      avatarPngDataUrl,
    }: {
      id: string;
      memoryLevel: SnapshotMemoryLevel;
      format: SnapshotFormat;
      memorySourcePubkey?: string | null;
      avatarPngDataUrl?: string;
    }) =>
      encodeAgentSnapshotForSend(
        id,
        memoryLevel,
        format,
        memorySourcePubkey,
        avatarPngDataUrl,
      ),
  });
}

export function usePreviewAgentSnapshotImportMutation() {
  return useMutation({
    mutationFn: ({
      fileBytes,
      fileName,
    }: {
      fileBytes: number[];
      fileName: string;
    }) => previewAgentSnapshotImport(fileBytes, fileName),
  });
}

export function useConfirmAgentSnapshotImportMutation() {
  return useMutation({
    mutationFn: (input: AgentSnapshotImportConfirm) =>
      confirmAgentSnapshotImport(input),
  });
}

// Re-export import types for consumers that import from hooks.
export type {
  AgentSnapshotImportPreview,
  AgentSnapshotImportConfirm,
  AgentSnapshotImportResult,
  EncodedSnapshotPayload,
};
