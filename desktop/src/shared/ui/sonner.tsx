import { Toaster as Sonner, useSonner } from "sonner";

import { ParkNativeWebviewsWhileMounted } from "@/shared/ui/ParkNativeWebviewsWhileMounted";

import { useTheme } from "@/shared/theme/ThemeProvider";

type ToasterProps = React.ComponentProps<typeof Sonner>;

const Toaster = ({ ...props }: ToasterProps) => {
  const { isDark } = useTheme();
  // Toasts render in HTML under native pin / playground webviews; park those
  // while any toast is on screen so it is never hidden behind a site.
  const { toasts } = useSonner();

  return (
    <>
    {toasts.length > 0 ? <ParkNativeWebviewsWhileMounted /> : null}
    <Sonner
      theme={isDark ? "dark" : "light"}
      className="toaster group"
      toastOptions={{
        classNames: {
          toast:
            "group toast group-[.toaster]:bg-background group-[.toaster]:text-foreground group-[.toaster]:border-border group-[.toaster]:shadow-lg",
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
