// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT, useUiLanguage } from "@/shared/i18n/uiTranslation";
import { Toaster as Sonner } from "sonner";

import { useTheme } from "@/shared/theme/ThemeProvider";

type ToasterProps = React.ComponentProps<typeof Sonner>;

const Toaster = ({ ...props }: ToasterProps) => {
  useUiLanguage();

  const { isDark } = useTheme();

  return (
    <Sonner
      containerAriaLabel={uiT("Notifications")}
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
  );
};

export { Toaster };
