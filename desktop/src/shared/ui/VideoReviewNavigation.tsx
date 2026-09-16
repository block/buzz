import * as React from "react";

type OpenVideoReview = (seconds: number) => void;

type VideoReviewNavigationValue = {
  open: (rootEventId: string, seconds: number) => void;
  register: (
    rootEventId: string,
    attachmentKey: string,
    handler: OpenVideoReview,
  ) => () => void;
};

const VideoReviewNavigationContext =
  React.createContext<VideoReviewNavigationValue | null>(null);

export function VideoReviewNavigationProvider({
  children,
}: {
  children: React.ReactNode;
}) {
  // Review comments are scoped to their message, not an individual attachment.
  // Keep attachment registration order so a multi-video message always opens
  // its first video instead of whichever player happened to update last.
  const handlersRef = React.useRef(
    new Map<string, Map<string, Set<OpenVideoReview>>>(),
  );
  const value = React.useMemo<VideoReviewNavigationValue>(
    () => ({
      open(rootEventId, seconds) {
        handlersRef.current
          .get(rootEventId)
          ?.values()
          .next()
          .value?.values()
          .next()
          .value?.(seconds);
      },
      register(rootEventId, attachmentKey, handler) {
        let rootHandlers = handlersRef.current.get(rootEventId);
        if (!rootHandlers) {
          rootHandlers = new Map();
          handlersRef.current.set(rootEventId, rootHandlers);
        }
        let attachmentHandlers = rootHandlers.get(attachmentKey);
        if (!attachmentHandlers) {
          attachmentHandlers = new Set();
          rootHandlers.set(attachmentKey, attachmentHandlers);
        }
        attachmentHandlers.add(handler);
        return () => {
          const registeredHandlers = handlersRef.current.get(rootEventId);
          const registeredAttachmentHandlers =
            registeredHandlers?.get(attachmentKey);
          registeredAttachmentHandlers?.delete(handler);
          if (registeredAttachmentHandlers?.size === 0) {
            registeredHandlers?.delete(attachmentKey);
          }
          if (registeredHandlers?.size === 0) {
            handlersRef.current.delete(rootEventId);
          }
        };
      },
    }),
    [],
  );

  return (
    <VideoReviewNavigationContext.Provider value={value}>
      {children}
    </VideoReviewNavigationContext.Provider>
  );
}

export function useOpenVideoReviewAt():
  | VideoReviewNavigationValue["open"]
  | null {
  return React.useContext(VideoReviewNavigationContext)?.open ?? null;
}

export function useRegisterVideoReview(
  reviewContext: { rootEventId?: string } | undefined,
  attachmentKey: string,
  handler: OpenVideoReview,
): void {
  const navigation = React.useContext(VideoReviewNavigationContext);
  const rootEventId = reviewContext?.rootEventId;
  const handlerRef = React.useRef(handler);
  React.useLayoutEffect(() => {
    handlerRef.current = handler;
  }, [handler]);
  const registeredHandler = React.useCallback(
    (seconds: number) => handlerRef.current(seconds),
    [],
  );

  React.useEffect(() => {
    if (!navigation || !rootEventId) return;
    return navigation.register(rootEventId, attachmentKey, registeredHandler);
  }, [attachmentKey, navigation, registeredHandler, rootEventId]);
}
