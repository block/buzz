import * as React from "react";

type FilePickerOptions = {
  accept?: string;
  multiple?: boolean;
};

/**
 * Owns one mounted file input for the hook lifetime. Reusing the node avoids
 * detached-input presentation races when a native picker is canceled and
 * immediately reopened.
 */
export function useFilePicker() {
  const inputRef = React.useRef<HTMLInputElement | null>(null);

  React.useEffect(
    () => () => {
      const input = inputRef.current;
      if (input) {
        input.onchange = null;
        input.remove();
      }
      inputRef.current = null;
    },
    [],
  );

  return React.useCallback(
    (options: FilePickerOptions, onFiles: (files: File[]) => void) => {
      let input = inputRef.current;
      if (!input) {
        input = document.createElement("input");
        input.type = "file";
        input.hidden = true;
        document.body.append(input);
        inputRef.current = input;
      }

      // Cancel emits no `change`, so replace rather than stack callbacks. Reset
      // before opening (and after selection) to permit choosing the same file.
      input.accept = options.accept ?? "";
      input.multiple = options.multiple ?? false;
      input.value = "";
      input.onchange = (event) => {
        const currentInput = event.currentTarget as HTMLInputElement;
        const files = Array.from(currentInput.files ?? []);
        currentInput.value = "";
        onFiles(files);
      };
      input.click();
    },
    [],
  );
}
