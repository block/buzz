import { Field } from "@base-ui/react/field";
import { Input } from "@base-ui/react/input";
import type { ComponentProps } from "react";

export type TextFieldProps = Omit<ComponentProps<typeof Input>, "className"> & {
  label: string;
  description?: string;
  error?: string;
};

/**
 * A labeled, single-line form control with Buzz appearance and Base UI
 * behavior. Search and other purpose-built controls remain separate
 * compositions because their icons and actions carry additional meaning.
 */
export function TextField({
  label,
  description,
  error,
  disabled,
  name,
  type = "text",
  ...inputProps
}: TextFieldProps) {
  return (
    <Field.Root
      className="text-field"
      disabled={disabled}
      invalid={Boolean(error)}
      name={name}
    >
      <Field.Label className="text-field-label text-body-sm text-primary font-semibold">
        {label}
      </Field.Label>
      <Input
        {...inputProps}
        className="text-field-control text-body"
        disabled={disabled}
        type={type}
      />
      {description ? (
        <Field.Description className="text-field-description text-body-sm text-tertiary">
          {description}
        </Field.Description>
      ) : null}
      {error ? (
        <Field.Error className="text-field-error text-body-sm" match>
          {error}
        </Field.Error>
      ) : null}
    </Field.Root>
  );
}
