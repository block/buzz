import { Field, Input, Textarea } from "@buzz/ui";

/** Field states use the production field and input semantics. */
export function FieldStates({
  kind = "input",
}: {
  kind?: "input" | "textarea";
}) {
  const Control = kind === "textarea" ? Textarea : Input;
  return (
    <div className="component-state-grid">
      {(["Empty", "Filled", "Invalid", "Read-only", "Disabled"] as const).map(
        (state) => (
          <Field.Root
            key={state}
            invalid={state === "Invalid"}
            disabled={state === "Disabled"}
          >
            <Field.Label>{state}</Field.Label>
            <Control
              placeholder="A place to build"
              defaultValue={state === "Empty" ? "" : "Build something together"}
              readOnly={state === "Read-only"}
              disabled={state === "Disabled"}
              aria-invalid={state === "Invalid" || undefined}
            />
            {state === "Invalid" && (
              <Field.Error match>Choose a different project name.</Field.Error>
            )}
          </Field.Root>
        ),
      )}
    </div>
  );
}

export function TextareaStates() {
  return <FieldStates kind="textarea" />;
}
