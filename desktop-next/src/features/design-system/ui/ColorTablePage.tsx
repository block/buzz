import { Link } from "@tanstack/react-router";
import { Fragment } from "react";

import {
  describeValueKind,
  humanizeVariable,
  isHex,
  useResolvedTokens,
} from "@/features/design-system/useResolvedToken";
import { RAMPS, ROLE_GROUPS } from "@/shared/tokens/registry";

import { Note, PageHeader, Section } from "./primitives";

/**
 * Every token in one table: the name you type, the base token it resolves
 * through, and the value it actually paints.
 *
 * The values are read from the live cascade rather than typed into the registry,
 * so this table reports what the product would really render and cannot drift
 * from `tokens.css`. It also means it re-resolves in dark mode, where the same
 * names hold different values — which is the whole point of the role layer.
 */

interface TableRow {
  /** The Tailwind class where one exists, else the custom property. */
  token: string;
  variable: string;
  /** Which layer this row belongs to. */
  layer: "role" | "ramp";
  group: string;
}

function collectRows(): TableRow[] {
  const rows: TableRow[] = [];

  for (const group of ROLE_GROUPS) {
    for (const role of group.roles) {
      rows.push({
        token: role.token,
        variable: role.variable,
        layer: "role",
        group: group.name,
      });
    }
  }

  for (const ramp of RAMPS) {
    for (const step of ramp.steps) {
      rows.push({
        token: step.variable.replace(/^--/, ""),
        variable: step.variable,
        layer: "ramp",
        group: `${ramp.name} ramp`,
      });
    }
  }

  return rows;
}

function ValueSwatch({ value }: { value: string }) {
  return (
    <span
      aria-hidden="true"
      className="inline-block h-4 w-4 shrink-0 rounded border border-tertiary align-middle"
      style={{ background: value }}
    />
  );
}

export function ColorTablePage() {
  const rows = collectRows();
  const resolved = useResolvedTokens(rows.map((row) => row.variable));

  const roleRows = rows.filter((row) => row.layer === "role");
  const rampRows = rows.filter((row) => row.layer === "ramp");

  return (
    <>
      <PageHeader
        title="Token table"
        intro="Every colour token in one list: the name you type, the base token it resolves through, and the value it actually paints. Values are read from the live cascade rather than written down, so this table cannot drift from the system — and it re-resolves when you switch modes."
      />

      <Note>
        Roles are the only layer a screen may use. Ramp steps are listed
        underneath so you can see what a role resolves through, but a component
        referencing one directly is a bug. The reasoning behind each name lives
        on the{" "}
        <Link to="/design/color" className="text-accent underline">
          colour page
        </Link>
        .
      </Note>

      <Section
        title="Roles"
        description="Grouped as they are in the system. The base column is what the role points at; the value column is where that chain ends."
      >
        <TokenTable rows={roleRows} resolved={resolved} showGroups />
      </Section>

      <Section
        title="Ramp steps"
        description="Layer 1. These hold the literal values every role resolves to, which is why the base column is empty for them."
      >
        <TokenTable rows={rampRows} resolved={resolved} showGroups />
      </Section>
    </>
  );
}

function TokenTable({
  rows,
  resolved,
  showGroups,
}: {
  rows: TableRow[];
  resolved: ReturnType<typeof useResolvedTokens>;
  showGroups?: boolean;
}) {
  let lastGroup: string | null = null;

  return (
    /* Scrolls horizontally rather than wrapping cells: a table of values is
       unreadable once a hex breaks across two lines. */
    <div className="-mx-1 overflow-x-auto px-1">
      <table className="w-full min-w-[34rem] border-collapse text-left">
        <caption className="sr-only">
          Colour tokens, the base token each resolves through, and the value it
          paints
        </caption>
        <thead>
          <tr className="border-tertiary border-b">
            <th scope="col" className="py-2 pr-4 text-label text-tertiary">
              Token
            </th>
            <th scope="col" className="py-2 pr-4 text-label text-tertiary">
              Base
            </th>
            <th scope="col" className="py-2 text-label text-tertiary">
              Value
            </th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => {
            const token = resolved.get(row.variable);
            const headingRow =
              showGroups && row.group !== lastGroup ? row.group : null;
            lastGroup = row.group;

            return (
              <Fragment key={row.variable}>
                {headingRow ? (
                  <tr>
                    <th
                      scope="colgroup"
                      colSpan={3}
                      className="pt-6 pb-1 text-meta text-tertiary"
                    >
                      {headingRow}
                    </th>
                  </tr>
                ) : null}
                <tr className="border-tertiary border-b">
                  <td className="py-2.5 pr-4 align-top whitespace-nowrap">
                    <code className="text-code text-primary">{row.token}</code>
                  </td>
                  <td className="py-2.5 pr-4 align-top whitespace-nowrap">
                    {token?.pointsAtVariable ? (
                      <code className="text-code text-secondary">
                        {humanizeVariable(token.pointsAtVariable)}
                      </code>
                    ) : (
                      <span className="text-caption text-tertiary">—</span>
                    )}
                  </td>
                  <td className="py-2.5 align-top">
                    <span className="flex min-w-0 items-center gap-2">
                      <ValueSwatch value={token?.value ?? "transparent"} />
                      <code className="truncate text-code text-secondary">
                        {token?.value ?? "…"}
                      </code>
                      {token && !isHex(token.value) ? (
                        <span className="shrink-0 text-meta text-tertiary">
                          {describeValueKind(token.value)}
                        </span>
                      ) : null}
                    </span>
                  </td>
                </tr>
              </Fragment>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
