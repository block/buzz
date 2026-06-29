import * as React from "react";
import { toast } from "sonner";

type UseProfileDmActionOptions = {
  effectivePubkey: string | null;
  onClose: () => void;
  onOpenDm?: (pubkeys: string[]) => Promise<void> | void;
};

export function useProfileDmAction({
  effectivePubkey,
  onClose,
  onOpenDm,
}: UseProfileDmActionOptions) {
  const isMountedRef = React.useRef(false);
  const [isOpeningDm, setIsOpeningDm] = React.useState(false);

  React.useEffect(() => {
    isMountedRef.current = true;
    return () => {
      isMountedRef.current = false;
    };
  }, []);

  const handleMessage = React.useCallback(async () => {
    if (!effectivePubkey || !onOpenDm || isOpeningDm) return;

    setIsOpeningDm(true);

    try {
      await onOpenDm([effectivePubkey]);
    } catch (error) {
      if (!isMountedRef.current) return;
      toast.error(
        error instanceof Error
          ? error.message
          : "Failed to open direct message.",
      );
      setIsOpeningDm(false);
      return;
    }

    if (!isMountedRef.current) return;
    setIsOpeningDm(false);
    onClose();
  }, [effectivePubkey, isOpeningDm, onClose, onOpenDm]);

  return { handleMessage, isOpeningDm };
}
