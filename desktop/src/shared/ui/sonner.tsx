import { Toaster as Sonner } from "sonner";

import { useTheme } from "@/shared/theme/ThemeProvider";
import { STATUS_TIMING, STATUS_Z } from "@/shared/ui/statusPills";

type ToasterProps = React.ComponentProps<typeof Sonner>;

const IS_WEB = import.meta.env?.VITE_BUZZ_PLATFORM === "web";
// Web: toasts follow the shared popup policy (statusPills.ts): same solid themed card, top-center under the nav row and
// tab pill, NOTE_MS on screen, no red text for errors.
const WEB_TOP = "calc(var(--phone-safe-top, env(safe-area-inset-top, 0px)) + 48px)";

const Toaster = ({ ...props }: ToasterProps) => {
  const { isDark } = useTheme();
  // Toasts never park native site webviews: hosts yield a bottom strip
  // instead (see shared/ui/toastStrip.ts).

  return (
    <>
    <Sonner
      theme={isDark ? "dark" : "light"}
      className="toaster group"
      {...(IS_WEB
        ? {
            position: "top-center" as const,
            offset: { top: WEB_TOP },
            mobileOffset: { top: WEB_TOP, left: "8px", right: "8px" },
            duration: STATUS_TIMING.NOTE_MS,
            style: { zIndex: STATUS_Z },
          }
        : {})}
      toastOptions={{
        classNames: {
          toast: IS_WEB
            ? "group toast group-[.toaster]:!bg-card group-[.toaster]:!text-card-foreground group-[.toaster]:!border-border group-[.toaster]:!rounded-[14px] group-[.toaster]:shadow-lg"
            : "group toast group-[.toaster]:bg-background group-[.toaster]:text-foreground group-[.toaster]:border-border group-[.toaster]:shadow-lg",
          ...(IS_WEB
            ? { error: "group-[.toaster]:!text-card-foreground [&_[data-icon]]:!text-muted-foreground" }
            : {}),
          description: "group-[.toast]:text-muted-foreground",
          actionButton:
            "group-[.toast]:bg-primary group-[.toast]:text-primary-foreground",
          cancelButton:
            "group-[.toast]:bg-muted group-[.toast]:text-muted-foreground",
        },
      }}
      {...props}
    />
    </>
  );
};

export { Toaster };
