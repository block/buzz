import { Input } from "@/shared/ui/input";
import { cn } from "@/shared/lib/cn";
import { RequiredFieldLabel } from "./agentConfigControls";
import {
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
} from "./agentConfigOptions";

export const OPENAI_COMPAT_BASE_URL = "OPENAI_COMPAT_BASE_URL";

function isSafeHttpBaseUrl(url: URL): boolean {
  return (
    (url.protocol === "http:" || url.protocol === "https:") &&
    url.host.length > 0 &&
    url.username.length === 0 &&
    url.password.length === 0 &&
    url.search.length === 0 &&
    url.hash.length === 0
  );
}

export function openAiCompatibleBaseUrlError(value: string): string | null {
  const trimmed = value.trim();
  if (trimmed.length === 0) return "Base URL is required.";
  try {
    if (isSafeHttpBaseUrl(new URL(trimmed))) return null;
  } catch {
    // Fall through to the actionable validation message.
  }
  return "Enter an HTTP or HTTPS URL without credentials, query, or fragment.";
}

export function OpenAiCompatibleBaseUrlField({
  disabled,
  inherited = false,
  onValueChange,
  value,
}: {
  disabled: boolean;
  inherited?: boolean;
  onValueChange: (next: string) => void;
  value: string;
}) {
  const error =
    inherited && value.trim().length === 0
      ? null
      : openAiCompatibleBaseUrlError(value);

  return (
    <div className="space-y-1.5">
      <RequiredFieldLabel htmlFor="openai-compatible-base-url" isRequired>
        Base URL
      </RequiredFieldLabel>
      <p className="text-xs text-muted-foreground font-mono">
        {OPENAI_COMPAT_BASE_URL}
      </p>
      <div
        className={cn(
          "flex min-h-11 items-center px-3",
          PERSONA_FIELD_SHELL_CLASS,
        )}
      >
        <Input
          aria-invalid={error !== null}
          autoCapitalize="none"
          autoCorrect="off"
          className={cn("h-8 px-0 py-0 leading-6", PERSONA_FIELD_CONTROL_CLASS)}
          data-testid="openai-compatible-base-url"
          disabled={disabled}
          id="openai-compatible-base-url"
          onBlur={(event) =>
            onValueChange(event.target.value.trim().replace(/\/+$/, ""))
          }
          onChange={(event) => onValueChange(event.target.value)}
          placeholder={inherited ? "Inherited" : "http://localhost:11434/v1"}
          type="url"
          value={value}
        />
      </div>
      {error ? <p className="text-xs text-destructive">{error}</p> : null}
    </div>
  );
}
