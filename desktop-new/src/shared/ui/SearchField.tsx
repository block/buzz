import { Field } from "@base-ui/react/field";
import { Input } from "@base-ui/react/input";
import { IconSearch, IconX } from "@tabler/icons-react";
import { IconButton } from "./IconButton";

export function SearchField({
  value,
  onValueChange,
  label = "Search",
  placeholder = "Search",
  variant = "default",
}: {
  value: string;
  onValueChange: (value: string) => void;
  label?: string;
  placeholder?: string;
  /** Navigator search uses the panel's broad corner to echo its enclosing surface. */
  variant?: "default" | "navigator";
}) {
  return (
    <Field.Root className="search-field" data-variant={variant}>
      <Field.Label className="sr-only">{label}</Field.Label>
      <IconSearch size={16} stroke={1.7} aria-hidden="true" />
      <Input
        type="search"
        value={value}
        onValueChange={onValueChange}
        placeholder={placeholder}
      />
      {value ? (
        <IconButton
          aria-label={`Clear ${label.toLowerCase()}`}
          icon={<IconX size={14} stroke={1.7} aria-hidden="true" />}
          size="compact"
          onClick={() => onValueChange("")}
        />
      ) : null}
    </Field.Root>
  );
}
