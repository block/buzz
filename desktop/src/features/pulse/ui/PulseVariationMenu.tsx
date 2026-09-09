import { MoreHorizontal } from "lucide-react";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";

export function PulseVariationMenu({
  value,
  onChange,
}: {
  value: "separate" | "combined";
  onChange: (value: "separate" | "combined") => void;
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          size="icon"
          variant="ghost"
          className="ml-1 h-8 w-8 shrink-0 rounded-full"
          aria-label="App variations"
          title="App variations"
        >
          <MoreHorizontal aria-hidden className="h-4 w-4" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuLabel>App variations</DropdownMenuLabel>
        <DropdownMenuRadioGroup
          value={value}
          onValueChange={(next) => {
            if (next === "separate" || next === "combined") onChange(next);
          }}
        >
          <DropdownMenuRadioItem value="separate">
            Separate feeds
          </DropdownMenuRadioItem>
          <DropdownMenuRadioItem value="combined">
            Combined conversations
          </DropdownMenuRadioItem>
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
