/**
 * Required-field label shared by agent dialogs.
 */
import type * as React from "react";

export function RequiredFieldLabel({
  children,
  htmlFor,
  isRequired,
}: {
  children: React.ReactNode;
  htmlFor: string;
  isRequired: boolean;
}) {
  return (
    <label className="text-sm font-medium" htmlFor={htmlFor}>
      {children}
      {isRequired ? (
        <span className="ml-1 text-destructive" aria-hidden="true">
          *
        </span>
      ) : null}
    </label>
  );
}
