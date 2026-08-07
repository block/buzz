import * as React from "react";

type OpenVideoReview = (seconds: number) => void;

type VideoReviewNavigationValue = {
  open: (rootEventId: string, seconds: number) => void;
  register: (rootEventId: string, handler: OpenVideoReview) => () => void;
};

const VideoReviewNavigationContext =
  React.createContext<VideoReviewNavigationValue | null>(null);

export function VideoReviewNavigationProvider({
  children,
}: {
  children: React.ReactNode;
}) {
  const handlersRef = React.useRef(new Map<string, OpenVideoReview>());
  const value = React.useMemo<VideoReviewNavigationValue>(
    () => ({
      open(rootEventId, seconds) {
        handlersRef.current.get(rootEventId)?.(seconds);
      },
      register(rootEventId, handler) {
        handlersRef.current.set(rootEventId, handler);
        return () => {
          if (handlersRef.current.get(rootEventId) === handler) {
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
  rootEventId: string | undefined,
  handler: OpenVideoReview,
): void {
  const navigation = React.useContext(VideoReviewNavigationContext);

  React.useEffect(() => {
    if (!navigation || !rootEventId) return;
    return navigation.register(rootEventId, handler);
  }, [handler, navigation, rootEventId]);
}
