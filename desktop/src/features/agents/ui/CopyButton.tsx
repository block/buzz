import { Copy } from "lucide-react";
import { toast } from "sonner";

import { Button, type ButtonProps } from "@/shared/ui/button";

export function CopyButton({
  className,
  label,
  size = "sm",
  value,
  variant = "outline",
}: {
  className?: string;
  label?: string;
  size?: ButtonProps["size"];
  value: string;
  variant?: ButtonProps["variant"];
}) {
  return (
    <Button
      className={className}
      onClick={async () => {
        await navigator.clipboard.writeText(value);
        toast.success("Copied to clipboard");
      }}
      size={size}
      type="button"
      variant={variant}
    >
      <Copy className="h-4 w-4" />
      <span>{label ?? "Copy"}</span>
    </Button>
  );
}
