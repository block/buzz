import { Moon, Sun, Monitor } from "lucide-react";
import { useTheme } from "@/shared/theme/ThemeProvider";
import { Button } from "@/shared/ui/button";

const icons = {
  light: Sun,
  dark: Moon,
  system: Monitor,
} as const;

const next: Record<string, "dark" | "system" | "light"> = {
  light: "dark",
  dark: "system",
  system: "light",
};

export function ThemeToggle() {
  const { theme, setTheme } = useTheme();
  const Icon = icons[theme];

  return (
    <Button
      variant="ghost"
      size="icon"
      className="h-8 w-8"
      onClick={() => setTheme(next[theme])}
      aria-label={`Theme: ${theme}. Click to switch.`}
    >
      <Icon className="h-4 w-4" />
    </Button>
  );
}
