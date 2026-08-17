import { cn } from "@/shared/lib/cn";
import type { TranscriptItem } from "../agentSessionTypes";
import type { CompactToolSummary } from "../agentSessionToolSummary";
import { useAgentSessionTranscriptVariant } from "../agentSessionTranscriptContext";
import {
  asRecord,
  formatTranscriptTimestampTitle,
  getToolString,
  parseToolResultValue,
} from "../agentSessionUtils";
import {
  ActivityRow,
  ActivityRowContent,
  ActivityRowLabel,
} from "../activityRenderClasses/ActivityRow";

type TodoDisplayItem = {
  checked: boolean;
  text: string;
};

export function TodoToolSummary({
  duration,
  fallbackPreview,
  item,
}: {
  duration: string | null;
  fallbackPreview: string | null;
  item: Extract<TranscriptItem, { type: "tool" }>;
}) {
  const todos = buildTodoDisplayItems(item.args, item.result, fallbackPreview);
  const variant = useAgentSessionTranscriptVariant();
  const isCompactPreview = variant === "compactPreview";
  const actionLabel = {
    verb: "Updated",
    object: fallbackPreview ?? "todos",
  };

  return (
    <ActivityRow title={formatTranscriptTimestampTitle(item.timestamp)}>
      <ActivityRowLabel
        object={actionLabel.object}
        openToneScope="tool"
        title={actionLabel.object}
        verb={actionLabel.verb}
      />
      {duration ? (
        <span className="shrink-0 text-xs text-muted-foreground/60 group-open:text-foreground">
          {duration}
        </span>
      ) : null}
      <ActivityRowContent className="pt-1 pb-1.5">
        {todos.length > 0 ? (
          <div className="space-y-1">
            {todos.map((todo, index) => (
              <TodoCheckboxRow
                // biome-ignore lint/suspicious/noArrayIndexKey: todo snapshots are static transcript content
                key={index}
                todo={todo}
              />
            ))}
          </div>
        ) : (
          <p
            className={cn(
              "text-muted-foreground/80",
              isCompactPreview ? "text-xs" : "text-sm",
            )}
          >
            No todos.
          </p>
        )}
      </ActivityRowContent>
    </ActivityRow>
  );
}

export function isTodoSummary(summary: CompactToolSummary) {
  return (
    summary.descriptor.groupKey === "plan:todo" ||
    summary.descriptor.operation === "todo"
  );
}

function TodoCheckboxRow({ todo }: { todo: TodoDisplayItem }) {
  const variant = useAgentSessionTranscriptVariant();
  const isCompactPreview = variant === "compactPreview";

  return (
    <div
      className={cn(
        "flex min-w-0 items-start gap-2 leading-5 text-muted-foreground",
        isCompactPreview ? "text-xs" : "text-sm",
      )}
    >
      <input
        checked={todo.checked}
        className="mt-0.5 h-3.5 w-3.5 shrink-0 cursor-default accent-primary"
        disabled
        readOnly
        type="checkbox"
      />
      <span className="min-w-0 wrap-break-word">{todo.text}</span>
    </div>
  );
}

function buildTodoDisplayItems(
  args: Record<string, unknown>,
  result: string,
  fallbackPreview: string | null,
): TodoDisplayItem[] {
  const argTodos = extractTodoItemsFromArgs(args);
  if (argTodos.length > 0) {
    return argTodos;
  }

  const resultTodos = extractTodoItemsFromResult(result);
  if (resultTodos.length > 0) {
    return resultTodos;
  }

  return fallbackPreview && fallbackPreview !== "empty list"
    ? [{ checked: false, text: fallbackPreview }]
    : [];
}

function extractTodoItemsFromArgs(
  args: Record<string, unknown>,
): TodoDisplayItem[] {
  const todos = args.todos;
  if (!Array.isArray(todos)) {
    return [];
  }

  return todos.flatMap((todo) => {
    if (!todo || typeof todo !== "object") {
      return [];
    }

    const record = asRecord(todo);
    const text = getToolString(record, ["text", "content", "label", "title"]);
    if (!text) {
      return [];
    }

    return [
      {
        checked: getTodoChecked(record),
        text,
      },
    ];
  });
}

function extractTodoItemsFromResult(result: string): TodoDisplayItem[] {
  const resultText = getTodoResultText(result);
  if (!resultText) {
    return [];
  }

  return resultText.split(/\r?\n/).flatMap((line) => {
    const match = line.match(/^\s*[-*]\s+\[([ xX])]\s+(.+?)\s*$/);
    if (!match) {
      return [];
    }

    return [
      {
        checked: match[1].toLowerCase() === "x",
        text: match[2],
      },
    ];
  });
}

function getTodoResultText(result: string): string {
  const parsed = parseToolResultValue(result);
  if (typeof parsed === "string") {
    return parsed;
  }

  const record = asRecord(parsed);
  return getToolString(record, ["stdout", "result", "text"]) ?? result;
}

function getTodoChecked(record: Record<string, unknown>) {
  if (typeof record.done === "boolean") {
    return record.done;
  }
  if (typeof record.checked === "boolean") {
    return record.checked;
  }

  const status = getToolString(record, ["status"])?.toLowerCase();
  return status === "completed" || status === "done";
}
