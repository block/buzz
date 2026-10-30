import * as React from "react";
import { toast } from "sonner";

import {
  isVoiceNoteAttachment,
  isVoiceNoteFile,
  VOICE_NOTE_MAX_DURATION_SECONDS,
} from "@/features/messages/lib/audioAttachment";
import { useVoiceNoteRecorder } from "@/features/messages/lib/useVoiceNoteRecorder";
import type { MediaUploadController } from "@/features/messages/lib/useMediaUpload";
import { VoiceNoteRecorder } from "./VoiceNoteRecorder";

export function useComposerVoiceNote({
  draftKey,
  editTargetId,
  media,
  setEmojiPickerOpen,
  setFormattingOpen,
}: {
  draftKey: string | null | undefined;
  editTargetId: string | null;
  media: MediaUploadController;
  setEmojiPickerOpen: (open: boolean) => void;
  setFormattingOpen: (open: boolean) => void;
}) {
  const recorder = useVoiceNoteRecorder();
  const limitReachedRef = React.useRef(false);
  const statusRef = React.useRef(recorder.status);
  statusRef.current = recorder.status;
  const getAttachments = React.useCallback(
    () => ({
      pending: media.pendingImetaRef.current,
      queued: media.queuedAttachmentsRef.current,
    }),
    [media.pendingImetaRef, media.queuedAttachmentsRef],
  );
  const getAttachmentsRef = React.useRef(getAttachments);
  getAttachmentsRef.current = getAttachments;
  const onBeforeStartRef = React.useRef(() => {});
  onBeforeStartRef.current = () => {
    setEmojiPickerOpen(false);
    setFormattingOpen(false);
  };
  const currentContextRef = React.useRef({ draftKey, editTargetId });
  currentContextRef.current = { draftKey, editTargetId };
  const recordingContextRef = React.useRef({ draftKey, editTargetId });

  // biome-ignore lint/correctness/useExhaustiveDependencies: composer identity fields are the cancellation triggers
  React.useEffect(() => {
    recorder.cancel();
  }, [draftKey, editTargetId]);

  React.useEffect(() => {
    if (recorder.error) toast.error(recorder.error);
  }, [recorder.error]);

  const finish = React.useCallback(async () => {
    const recording = await recorder.stop();
    const recordingContext = recordingContextRef.current;
    const currentContext = currentContextRef.current;
    if (
      recording &&
      recordingContext.draftKey === currentContext.draftKey &&
      recordingContext.editTargetId === currentContext.editTargetId
    ) {
      await media.uploadFile(recording.file);
    }
    return recording;
  }, [recorder.stop, media.uploadFile]);

  React.useEffect(() => {
    if (recorder.status === "idle") limitReachedRef.current = false;
    if (
      recorder.status === "recording" &&
      recorder.elapsedSeconds >= VOICE_NOTE_MAX_DURATION_SECONDS &&
      !limitReachedRef.current
    ) {
      limitReachedRef.current = true;
      void finish();
    }
  }, [finish, recorder.elapsedSeconds, recorder.status]);

  const toggle = React.useCallback(() => {
    if (statusRef.current === "recording") {
      void finish();
      return;
    }
    const attachments = getAttachmentsRef.current();
    if (attachments.pending.length > 0 || attachments.queued.length > 0) {
      toast.error("A voice note must be the only attachment.");
      return;
    }
    recordingContextRef.current = currentContextRef.current;
    onBeforeStartRef.current();
    void recorder.start();
  }, [finish, recorder.start]);

  const cancel = recorder.cancel;
  const attachments = getAttachments();
  const hasAttachment =
    attachments.pending.some((attachment) =>
      isVoiceNoteAttachment({
        filename: attachment.filename,
        m: attachment.type,
      }),
    ) || attachments.queued.some(({ file }) => isVoiceNoteFile(file));
  const hasAttachmentRef = React.useRef(hasAttachment);
  hasAttachmentRef.current = hasAttachment;

  const acceptsNewAttachment = React.useCallback(() => {
    const attachments = getAttachmentsRef.current();
    const hasVoiceNoteAttachment =
      attachments.pending.some((attachment) =>
        isVoiceNoteAttachment({
          filename: attachment.filename,
          m: attachment.type,
        }),
      ) || attachments.queued.some(({ file }) => isVoiceNoteFile(file));
    if (statusRef.current !== "idle" || hasVoiceNoteAttachment) {
      toast.error(
        statusRef.current === "idle"
          ? "A voice note must be the only attachment."
          : "Finish or discard the voice note before attaching a file.",
      );
      return false;
    }
    return true;
  }, []);

  const uploadFileWhenIdle = React.useCallback(
    async (file: File) => {
      if (!acceptsNewAttachment()) return;
      await media.uploadFile(file);
    },
    [acceptsNewAttachment, media.uploadFile],
  );
  const setPendingImetaWhenIdle = React.useCallback(
    (update: Parameters<typeof media.setPendingImeta>[0]) => {
      if (acceptsNewAttachment()) media.setPendingImeta(update);
    },
    [acceptsNewAttachment, media.setPendingImeta],
  );

  return {
    ...recorder,
    acceptsAttachment: recorder.status === "idle" && !hasAttachment,
    hasAttachment,
    hasAttachmentRef,
    isIdle: recorder.status === "idle",
    recorderElement:
      recorder.status === "idle" ? null : (
        <VoiceNoteRecorder
          elapsedSeconds={recorder.elapsedSeconds}
          levels={recorder.levels}
          maxDurationSeconds={VOICE_NOTE_MAX_DURATION_SECONDS}
          onCancel={cancel}
          processing={recorder.status === "processing"}
          requesting={recorder.status === "requesting"}
        />
      ),
    statusRef,
    setPendingImetaWhenIdle,
    finish,
    toggle,
    uploadFileWhenIdle,
  };
}
