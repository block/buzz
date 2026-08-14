import * as React from "react";

import type { BlobDescriptor } from "@/shared/api/tauri";
import {
  isAgentSnapshotPngFilename,
  withAgentSnapshotAvatarPreview,
} from "./agentSnapshotAttachmentPreview";

/** Recover composer avatars that are absent from older draft/clipboard state. */
export function useAgentSnapshotAvatarRecovery(
  attachments: BlobDescriptor[],
  setAttachmentSlots: React.Dispatch<
    React.SetStateAction<(BlobDescriptor | null)[]>
  >,
) {
  React.useEffect(() => {
    const candidates = attachments.filter(
      (attachment) =>
        !attachment.snapshotAvatarUrl &&
        isAgentSnapshotPngFilename(attachment.filename),
    );
    if (candidates.length === 0) return;

    let cancelled = false;
    void Promise.all(
      candidates.map((attachment) =>
        withAgentSnapshotAvatarPreview(attachment),
      ),
    ).then((resolved) => {
      if (cancelled) return;
      const avatarsByAttachment = new Map(
        resolved.flatMap((attachment) =>
          attachment.snapshotAvatarUrl
            ? [
                [
                  `${attachment.url}\u0000${attachment.sha256}`,
                  attachment.snapshotAvatarUrl,
                ] as const,
              ]
            : [],
        ),
      );
      if (avatarsByAttachment.size === 0) return;

      setAttachmentSlots((current) => {
        let changed = false;
        const next = current.map((attachment) => {
          if (!attachment || attachment.snapshotAvatarUrl) return attachment;
          const snapshotAvatarUrl = avatarsByAttachment.get(
            `${attachment.url}\u0000${attachment.sha256}`,
          );
          if (!snapshotAvatarUrl) return attachment;
          changed = true;
          return { ...attachment, snapshotAvatarUrl };
        });
        return changed ? next : current;
      });
    });

    return () => {
      cancelled = true;
    };
  }, [attachments, setAttachmentSlots]);
}
