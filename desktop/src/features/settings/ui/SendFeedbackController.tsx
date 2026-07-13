import { useSendFeedback } from "@/features/settings/hooks/useSendFeedback";
import { SendFeedbackDialog } from "@/features/settings/ui/SendFeedbackDialog";

/**
 * Kept behind the build-time feedback gate in AppShell so default OSS builds
 * neither render the UI nor start a second channels query for a capability they
 * do not offer.
 */
export function SendFeedbackController({
  onOpenChange,
  open,
}: {
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  const sendFeedback = useSendFeedback();
  if (!sendFeedback.destinationChannelId) {
    return null;
  }

  return (
    <SendFeedbackDialog
      attachedImageUrl={sendFeedback.attachedImage?.url ?? null}
      destinationChannelId={sendFeedback.destinationChannelId}
      destinationChannelName={sendFeedback.destinationChannelName}
      destinationRelayUrl={sendFeedback.destinationRelayUrl}
      destinationWorkspaceName={sendFeedback.destinationWorkspaceName}
      isPending={sendFeedback.isPending}
      onAttachImage={sendFeedback.attachImage}
      onOpenChange={(nextOpen) => {
        onOpenChange(nextOpen);
        if (!nextOpen) {
          sendFeedback.reset();
        }
      }}
      onRemoveImage={sendFeedback.removeImage}
      onSubmit={sendFeedback.submit}
      open={open}
    />
  );
}
