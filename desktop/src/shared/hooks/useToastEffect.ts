import * as React from "react";
import { toast } from "sonner";

/**
 * Show a toast when a message string becomes truthy. Uses a ref to avoid
 * double-firing in React StrictMode (where effects run twice with the same
 * value).
 */
function useToastEffect(
  message: string | null | undefined,
  variant: "success" | "error",
) {
  const shownRef = React.useRef<string | null>(null);

  React.useEffect(() => {
    if (message && message !== shownRef.current) {
      shownRef.current = message;
      toast[variant](message);
    }
    if (!message) {
      shownRef.current = null;
    }
  }, [message, variant]);
}

/**
 * Convenience wrapper: show success/error toasts for a pair of feedback
 * message strings (common pattern after mutations).
 */
export function useFeedbackToasts(
  noticeMessage: string | null | undefined,
  errorMessage: string | null | undefined,
) {
  useToastEffect(noticeMessage, "success");
  useToastEffect(errorMessage, "error");
}
