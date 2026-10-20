import * as React from "react";
import type { UseMutationResult } from "@tanstack/react-query";

import type { CatchUpSettings } from "@/shared/api/tauriThreadCatchUp";
import { Alert, AlertDescription } from "@/shared/ui/alert";
import { Button } from "@/shared/ui/button";
import { Checkbox } from "@/shared/ui/checkbox";
import { Input } from "@/shared/ui/input";
import { validateCatchUpSettings } from "./threadCatchUp";

/**
 * Explicit local-model setup. There is deliberately no API-key field: recaps
 * run only against a model on this computer, and native refuses any
 * provider credential in this mode.
 */
export function ThreadCatchUpSetup({
  initial,
  onCancel,
  onSaved,
  saveSettings,
}: {
  initial?: CatchUpSettings;
  onCancel?: () => void;
  onSaved: () => void;
  saveSettings: UseMutationResult<
    CatchUpSettings,
    Error,
    CatchUpSettings,
    unknown
  >;
}) {
  const [baseUrl, setBaseUrl] = React.useState(initial?.baseUrl ?? "");
  const [model, setModel] = React.useState(initial?.model ?? "");
  const [confirmedLocal, setConfirmedLocal] = React.useState(
    initial?.confirmedLocal ?? false,
  );
  const [submitted, setSubmitted] = React.useState(false);
  const draft = {
    baseUrl: baseUrl.trim(),
    model: model.trim(),
    confirmedLocal,
  };
  const validation = validateCatchUpSettings(draft);
  const formId = React.useId();

  return (
    <form
      aria-describedby={`${formId}-intro ${formId}-requirements`}
      className="space-y-3"
      data-testid="thread-catch-up-setup"
      noValidate
      onSubmit={(event) => {
        event.preventDefault();
        setSubmitted(true);
        if (validation) return;
        saveSettings.mutate(draft, { onSuccess: onSaved });
      }}
    >
      <p
        className="text-xs leading-5 text-muted-foreground"
        id={`${formId}-intro`}
      >
        Buzz sends this thread only to the endpoint you choose here, with no API
        key and no hosted fallback. Buzz does not install models; point it at
        one you already run.
      </p>
      <p
        className="text-2xs leading-4 text-muted-foreground"
        data-testid="thread-catch-up-setup-requirements"
        id={`${formId}-requirements`}
      >
        The server must support OpenAI-compatible JSON-object output, with its
        context window set to at least 32,768 tokens on the server.
      </p>
      <div className="space-y-1">
        <label className="text-xs font-medium" htmlFor={`${formId}-url`}>
          Local endpoint
        </label>
        <Input
          autoComplete="off"
          className="h-8 text-sm"
          id={`${formId}-url`}
          inputMode="url"
          onChange={(event) => setBaseUrl(event.target.value)}
          placeholder="http://127.0.0.1:11434/v1"
          spellCheck={false}
          value={baseUrl}
        />
      </div>
      <div className="space-y-1">
        <label className="text-xs font-medium" htmlFor={`${formId}-model`}>
          Model
        </label>
        <Input
          autoComplete="off"
          className="h-8 text-sm"
          id={`${formId}-model`}
          onChange={(event) => setModel(event.target.value)}
          placeholder="qwen3:4b"
          spellCheck={false}
          value={model}
        />
      </div>
      <div className="flex items-start gap-2">
        <Checkbox
          checked={confirmedLocal}
          className="mt-0.5"
          id={`${formId}-local`}
          onCheckedChange={(value) => setConfirmedLocal(value === true)}
        />
        <label className="text-xs leading-5" htmlFor={`${formId}-local`}>
          I confirm this endpoint runs the model on this computer and does not
          forward the thread to a hosted service.
        </label>
      </div>
      {submitted && validation ? (
        <p
          className="text-xs text-destructive"
          data-testid="thread-catch-up-setup-error"
          role="alert"
        >
          {validation}
        </p>
      ) : null}
      {saveSettings.isError ? (
        <Alert variant="destructive">
          <AlertDescription>
            Setup was not saved. {saveSettings.error.message}
          </AlertDescription>
        </Alert>
      ) : null}
      <div className="flex flex-wrap gap-2">
        <Button
          data-testid="thread-catch-up-setup-save"
          disabled={saveSettings.isPending}
          size="sm"
          type="submit"
        >
          {saveSettings.isPending ? "Saving…" : "Save setup"}
        </Button>
        {onCancel ? (
          <Button onClick={onCancel} size="sm" type="button" variant="ghost">
            Cancel
          </Button>
        ) : null}
      </div>
    </form>
  );
}
