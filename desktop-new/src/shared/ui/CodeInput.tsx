import {
  type ChangeEvent,
  type ClipboardEvent,
  type KeyboardEvent,
  useRef,
  useState,
} from "react";

export type CodeInputProps = {
  label: string;
  labelHidden?: boolean;
  defaultValue?: string;
  disabled?: boolean;
  onValueChange?: (value: string) => void;
};

const CODE_INPUT_LENGTH = 6;
const CODE_INPUT_DIGIT_KEYS = ["1", "2", "3", "4", "5", "6"] as const;

function digitsFrom(value: string): string[] {
  const digits = value.replace(/\D/g, "").slice(0, CODE_INPUT_LENGTH).split("");
  return Array.from(
    { length: CODE_INPUT_LENGTH },
    (_, index) => digits[index] ?? "",
  );
}

/**
 * A segmented numeric code field with one-time-code autofill, paste
 * distribution, and keyboard movement between digits.
 */
export function CodeInput({
  label,
  labelHidden = false,
  defaultValue = "",
  disabled = false,
  onValueChange,
}: CodeInputProps) {
  const [digits, setDigits] = useState(() => digitsFrom(defaultValue));
  const inputs = useRef<Array<HTMLInputElement | null>>([]);

  const update = (next: string[]) => {
    setDigits(next);
    onValueChange?.(next.join(""));
  };

  const insert = (index: number, value: string) => {
    const incoming = value
      .replace(/\D/g, "")
      .slice(0, CODE_INPUT_LENGTH - index);
    const next = [...digits];

    if (!incoming) {
      next[index] = "";
      update(next);
      return;
    }

    for (const [offset, digit] of [...incoming].entries()) {
      next[index + offset] = digit;
    }
    update(next);
    inputs.current[
      Math.min(index + incoming.length, CODE_INPUT_LENGTH - 1)
    ]?.focus();
  };

  const handleChange = (
    index: number,
    event: ChangeEvent<HTMLInputElement>,
  ) => {
    insert(index, event.target.value);
  };

  const handlePaste = (
    index: number,
    event: ClipboardEvent<HTMLInputElement>,
  ) => {
    const pasted = event.clipboardData.getData("text");
    if (!/\d/.test(pasted)) return;
    event.preventDefault();
    insert(index, pasted);
  };

  const handleKeyDown = (
    index: number,
    event: KeyboardEvent<HTMLInputElement>,
  ) => {
    if (event.key === "Backspace" && !digits[index] && index > 0) {
      event.preventDefault();
      const next = [...digits];
      next[index - 1] = "";
      update(next);
      inputs.current[index - 1]?.focus();
      return;
    }

    if (event.key === "ArrowLeft" && index > 0) {
      event.preventDefault();
      inputs.current[index - 1]?.focus();
    } else if (event.key === "ArrowRight" && index < CODE_INPUT_LENGTH - 1) {
      event.preventDefault();
      inputs.current[index + 1]?.focus();
    }
  };

  return (
    <fieldset className="code-input" disabled={disabled}>
      <legend
        className={`code-input-label text-body-sm text-primary font-semibold${labelHidden ? " sr-only" : ""}`}
      >
        {label}
      </legend>
      <div className="code-input-group">
        {digits.map((digit, index) => (
          <div className="code-input-slot" key={CODE_INPUT_DIGIT_KEYS[index]}>
            <input
              aria-label={`${label} digit ${index + 1} of ${CODE_INPUT_LENGTH}`}
              autoComplete={index === 0 ? "one-time-code" : "off"}
              className="code-input-control text-body"
              inputMode="numeric"
              maxLength={1}
              onChange={(event) => handleChange(index, event)}
              onFocus={(event) => event.currentTarget.select()}
              onKeyDown={(event) => handleKeyDown(index, event)}
              onPaste={(event) => handlePaste(index, event)}
              pattern="[0-9]*"
              ref={(element) => {
                inputs.current[index] = element;
              }}
              type="text"
              value={digit}
            />
            {digit ? (
              <span
                aria-hidden="true"
                className="code-input-digit text-body font-semibold"
                key={digit}
              >
                {digit}
              </span>
            ) : null}
          </div>
        ))}
      </div>
    </fieldset>
  );
}
