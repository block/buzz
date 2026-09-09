import { useMemo } from "react";
import { Check, Circle, CircleAlert, LoaderCircle } from "lucide-react";
import { Button } from "../button";
import { Alert, Badge, Card, Table } from "../surfaces";
import { parseGeneratedView, type GeneratedBlock } from "./schema";
/** Inputs to the renderer. Only the host's explicit action allowlist can become interactive. */
export interface GeneratedResponseProps {
  response: unknown;
  state?: "complete" | "streaming";
  actions?: Readonly<Record<string, () => void>>;
  pendingAction?: string;
  onRetry?: () => void;
}
const statuses = {
  pending: { Icon: Circle, tone: "neutral" },
  running: { Icon: LoaderCircle, tone: "accent" },
  complete: { Icon: Check, tone: "success" },
  failed: { Icon: CircleAlert, tone: "danger" },
} as const;
function Block({
  block,
  state,
  actions,
  pendingAction,
}: { block: GeneratedBlock } & Pick<
  GeneratedResponseProps,
  "state" | "actions" | "pendingAction"
>) {
  switch (block.type) {
    case "text":
      return (
        <p className="text-body text-secondary whitespace-pre-wrap">
          {block.text}
        </p>
      );
    case "notice":
      return <Alert tone={block.tone}>{block.text}</Alert>;
    case "metric":
      return (
        <div className="bui-stack">
          <p className="text-caption text-secondary">{block.label}</p>
          <p className="text-title">{block.value}</p>
          {block.detail && (
            <p className="text-caption text-secondary">{block.detail}</p>
          )}
        </div>
      );
    case "tasks":
      return (
        <ul className="bui-reset bui-stack">
          {block.items.map((item) => {
            const { Icon, tone } = statuses[item.status];
            return (
              <li key={item.id} className="bui-inline">
                <Icon aria-hidden="true" className="bui-icon" />
                <span className="text-body" style={{ flex: 1 }}>
                  {item.label}
                </span>
                <Badge tone={tone}>{item.status}</Badge>
              </li>
            );
          })}
        </ul>
      );
    case "table":
      return (
        <Table>
          <thead>
            <tr>
              {block.columns.map((column) => (
                <th key={column} scope="col">
                  {column}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {block.rows.map((row, index) => (
              // Rows are immutable values in a complete snapshot, with no local component state.
              // biome-ignore lint/suspicious/noArrayIndexKey: The schema permits identical rows and has no row identity.
              <tr key={index}>
                {row.map((cell, column) => (
                  <td key={block.columns[column]}>{cell}</td>
                ))}
              </tr>
            ))}
          </tbody>
        </Table>
      );
    case "actions":
      return (
        <div className="bui-inline">
          {block.items.map((item) => {
            const action =
              actions && Object.hasOwn(actions, item.id)
                ? actions[item.id]
                : undefined;
            return (
              <Button
                key={item.id}
                disabled={
                  state === "streaming" || !action || Boolean(pendingAction)
                }
                loading={pendingAction === item.id}
                onClick={() => action?.()}
              >
                {item.label}
              </Button>
            );
          })}
        </div>
      );
  }
}
/** Render a bounded generated snapshot using the same Buzz components as hand-authored screens. */
export function GeneratedResponse({
  response,
  state = "complete",
  actions,
  pendingAction,
  onRetry,
}: GeneratedResponseProps) {
  const result = useMemo(() => parseGeneratedView(response), [response]);
  if (!result.ok)
    return (
      <Alert tone="danger">
        <div className="bui-stack">
          <p className="text-label">This response could not be displayed</p>
          <p>{result.error}</p>
          {onRetry && (
            <Button variant="outline" onClick={onRetry}>
              Try again
            </Button>
          )}
        </div>
      </Alert>
    );
  return (
    <Card aria-busy={state === "streaming"}>
      <div className="bui-inline">
        <h3 className="text-heading" style={{ flex: 1 }}>
          {result.value.title}
        </h3>
        {state === "streaming" && <Badge>Updating</Badge>}
      </div>
      {result.value.blocks.map((block) => (
        <Block
          key={block.id}
          block={block}
          state={state}
          actions={actions}
          pendingAction={pendingAction}
        />
      ))}
    </Card>
  );
}
