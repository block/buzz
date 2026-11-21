import * as React from "react";
import { toast } from "sonner";

type ToastVariant = "success" | "error";

/**
 * Errors are the one class of feedback still needed after it is read: they run
 * to several lines and are often worth copying into a bug report. Sonner's
 * four-second default races that, so an error stays up until it is dismissed
 * and carries an explicit close control. A success notice says nothing that
 * outlives the glance, so it keeps the default treatment.
 */
export function feedbackToastOptions(variant: ToastVariant) {
  return variant === "error"
    ? { closeButton: true, duration: Number.POSITIVE_INFINITY }
    : undefined;
}

/**
 * Show a toast when a message string becomes truthy. Uses a ref to avoid
 * double-firing in React StrictMode (where effects run twice with the same
 * value).
 */
function useToastEffect(
  message: string | null | undefined,
  variant: ToastVariant,
) {
  const shownRef = React.useRef<string | null>(null);

  React.useEffect(() => {
    if (message && message !== shownRef.current) {
      shownRef.current = message;
      toast[variant](message, feedbackToastOptions(variant));
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
