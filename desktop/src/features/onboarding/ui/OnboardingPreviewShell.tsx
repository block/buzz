import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { Card } from "@/shared/ui/card";
import { useSmoothCorners } from "@/shared/ui/smoothCorners";
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";
import { OnboardingChrome } from "./OnboardingChrome";
import { OnboardingFooterProvider } from "./OnboardingFooter";

const OnboardingPreviewCardContext = React.createContext(false);

export function OnboardingPreviewLayoutProvider({
  card,
  children,
}: {
  card: boolean;
  children: React.ReactNode;
}) {
  return (
    <OnboardingPreviewCardContext.Provider value={card}>
      {children}
    </OnboardingPreviewCardContext.Provider>
  );
}

/** Whether the current workshop screen uses the experimental V3 card layout. */
export function useOnboardingPreviewCardLayout() {
  return React.useContext(OnboardingPreviewCardContext);
}

export function OnboardingPreviewStep({
  allowWideContent = false,
  allowHorizontalActionOverflow = false,
  children,
  current = 2,
  onBack,
  security = false,
  testId,
  total,
}: {
  allowWideContent?: boolean;
  allowHorizontalActionOverflow?: boolean;
  children: React.ReactNode;
  current?: number;
  onBack?: () => void;
  security?: boolean;
  testId: string;
  total?: number;
}) {
  const cardLayout = useOnboardingPreviewCardLayout();
  const cardRef = React.useRef<HTMLDivElement | null>(null);
  useSmoothCorners(cardRef, { enabled: cardLayout });
  const frame = (
    <div
      className={cn(
        "buzz-onboarding-step-frame relative flex w-full flex-col",
        cardLayout
          ? cn(
              "min-h-0 max-w-none flex-1 items-stretch overflow-x-hidden overflow-y-auto overscroll-contain text-left",
              allowHorizontalActionOverflow
                ? "-mx-6 w-[calc(100%+3rem)] px-6"
                : "-mx-2 w-[calc(100%+1rem)] px-2",
            )
          : "max-w-[1040px] flex-1 items-center text-center",
      )}
    >
      {children}
    </div>
  );

  return (
    <div
      className={cn(
        "buzz-onboarding-neutral-theme buzz-startup-shell flex max-h-dvh justify-center overflow-x-hidden overflow-y-auto px-4 text-foreground",
        cardLayout ? "items-center py-6" : "items-start pb-28 pt-[106px]",
        security && !cardLayout && "buzz-onboarding-security-theme",
      )}
      data-testid={testId}
    >
      <StartupWindowDragRegion />
      {!security || cardLayout ? (
        <OnboardingChrome current={current} total={total} />
      ) : null}
      {cardLayout ? (
        <Card
          className={cn(
            "flex h-[min(41.5rem,calc(100dvh-3rem))] w-max min-w-[calc(38rem+2px)] max-w-[50rem] flex-col overflow-hidden rounded-[2rem] bg-white p-12 text-left shadow-lg [--buzz-onboarding-cta-label:#fff] [&_.buzz-onboarding-transition-content]:min-w-[32rem] [&_.buzz-onboarding-transition-content]:!text-left [&_.buzz-onboarding-transition-line]:justify-start [&_h1+p]:!mx-0 [&_h1+p]:!mt-2 [&_h1+p]:!text-left [&_h1+p]:!text-base [&_h1+p]:!leading-6 [&_h1]:!text-left [&_h1]:!text-2xl [&_h1]:!leading-8 [&_h1]:!text-foreground",
            !allowWideContent &&
              "[&_.buzz-onboarding-transition-content]:max-w-[32rem]",
          )}
          data-testid="onboarding-preview-content-card"
          ref={cardRef}
        >
          <OnboardingFooterProvider
            backAction={onBack ? { onClick: onBack } : undefined}
            placement="card"
          >
            {frame}
          </OnboardingFooterProvider>
        </Card>
      ) : (
        <OnboardingFooterProvider
          backAction={onBack ? { onClick: onBack } : undefined}
        >
          {frame}
        </OnboardingFooterProvider>
      )}
    </div>
  );
}
