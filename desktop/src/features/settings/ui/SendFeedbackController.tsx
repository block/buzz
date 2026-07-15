import { useSendFeedback } from "@/features/settings/hooks/useSendFeedback";
import { SendFeedbackDialog } from "@/features/settings/ui/SendFeedbackDialog";

export function SendFeedbackController({
  onOpenChange,
  open,
}: {
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  const sendFeedback = useSendFeedback();
  return (
    <SendFeedbackDialog
      attachedImageUrl={sendFeedback.attachedImage?.url ?? null}
      isAttaching={sendFeedback.isAttaching}
      isPending={sendFeedback.isPending}
      onAttachImage={sendFeedback.attachImage}
      onOpenChange={(nextOpen) => {
        onOpenChange(nextOpen);
        if (!nextOpen) sendFeedback.reset();
      }}
      onRemoveImage={sendFeedback.removeImage}
      onSubmit={sendFeedback.submit}
      open={open}
    />
  );
}
