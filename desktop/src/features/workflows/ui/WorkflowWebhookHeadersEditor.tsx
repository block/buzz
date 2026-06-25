import { Plus, Trash2 } from "lucide-react";

import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { FieldLabel } from "./workflowFormPrimitives";
import type { HeaderFormState } from "./workflowFormTypes";

function updateHeaders(
  headers: HeaderFormState[],
  updater: (headers: HeaderFormState[]) => HeaderFormState[],
) {
  return updater(headers);
}

type WorkflowWebhookHeadersEditorProps = {
  disabled?: boolean;
  headers: HeaderFormState[];
  onChange: (headers: HeaderFormState[]) => void;
  stepId: string;
};

export function WorkflowWebhookHeadersEditor({
  disabled,
  headers,
  onChange,
  stepId,
}: WorkflowWebhookHeadersEditorProps) {
  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <FieldLabel>Headers (optional)</FieldLabel>
        <Button
          className="h-7 gap-1 text-xs"
          disabled={disabled}
          onClick={() =>
            onChange(
              updateHeaders(headers, (currentHeaders) => [
                ...currentHeaders,
                {
                  id: `${stepId}_header_${currentHeaders.length + 1}`,
                  key: "",
                  value: "",
                },
              ]),
            )
          }
          size="sm"
          type="button"
          variant="outline"
        >
          <Plus className="h-4 w-4" />
          Add header
        </Button>
      </div>
      {headers.length === 0 ? (
        <p className="text-xs text-muted-foreground">No custom headers.</p>
      ) : (
        <div className="space-y-2">
          {headers.map((header, index) => (
            <div className="flex items-center gap-2" key={header.id}>
              <Input
                autoCapitalize="off"
                disabled={disabled}
                onChange={(event) =>
                  onChange(
                    updateHeaders(headers, (currentHeaders) =>
                      currentHeaders.map((current, currentIndex) =>
                        currentIndex === index
                          ? { ...current, key: event.target.value }
                          : current,
                      ),
                    ),
                  )
                }
                placeholder="Header name"
                value={header.key}
              />
              <Input
                autoCapitalize="off"
                disabled={disabled}
                onChange={(event) =>
                  onChange(
                    updateHeaders(headers, (currentHeaders) =>
                      currentHeaders.map((current, currentIndex) =>
                        currentIndex === index
                          ? { ...current, value: event.target.value }
                          : current,
                      ),
                    ),
                  )
                }
                placeholder="Header value"
                value={header.value}
              />
              <Button
                aria-label="Remove header"
                className="h-9 w-9 shrink-0"
                disabled={disabled}
                onClick={() =>
                  onChange(
                    updateHeaders(headers, (currentHeaders) =>
                      currentHeaders.filter(
                        (_, currentIndex) => currentIndex !== index,
                      ),
                    ),
                  )
                }
                size="icon"
                type="button"
                variant="ghost"
              >
                <Trash2 className="h-4 w-4 text-muted-foreground" />
              </Button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
