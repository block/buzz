import { Copy } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/shared/ui/button";

export function CopyButton({
  value,
  label,
}: {
  value: string;
  label?: string;
}) {
  return (
    <Button
      onClick={async () => {
        await navigator.clipboard.writeText(value);
        toast.success("Copied to clipboard");
      }}
      size="sm"
      type="button"
      variant="outline"
    >
      <Copy className="h-4 w-4" />
      <span>{label ?? "Copy"}</span>
    </Button>
  );
}
