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
    /* `table-fixed` with three equal columns: the natural `auto` layout gives
       the value column most of the width, because one gradient literal is longer
       than every other cell in the table combined. Fixed makes the thirds hold
       regardless of content, and cells wrap instead of scrolling. */
    <table className="w-full table-fixed border-collapse text-left">
      <caption className="sr-only">
        Colour tokens, the base token each resolves through, and the value it
        paints
      </caption>
      <colgroup>
        <col className="w-1/3" />
        <col className="w-1/3" />
        <col className="w-1/3" />
      </colgroup>
      <thead>
        <tr className="border-tertiary border-b">
          <th scope="col" className="py-2 pr-4 text-body text-tertiary">
            Token
          </th>
          <th scope="col" className="py-2 pr-4 text-body text-tertiary">
            Base
          </th>
          <th scope="col" className="py-2 text-body text-tertiary">
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
                    className="pt-6 pb-1 text-body-sm text-tertiary"
                  >
                    {headingRow}
                  </th>
                </tr>
              ) : null}
              <tr className="border-tertiary border-b">
                <td className="py-2.5 pr-4 align-top">
                  <code className="break-words text-mono text-primary">
                    {row.token}
                  </code>
                </td>
                <td className="py-2.5 pr-4 align-top">
                  {token?.pointsAtVariable ? (
                    <code className="break-words text-mono text-secondary">
                      {humanizeVariable(token.pointsAtVariable)}
                    </code>
                  ) : (
                    <span className="text-body-sm text-tertiary">—</span>
                  )}
                </td>
                <td className="py-2.5 align-top">
                  <span className="flex min-w-0 items-start gap-2">
                    <span className="mt-0.5 shrink-0">
                      <ValueSwatch value={token?.value ?? "transparent"} />
                    </span>
                    <span className="flex min-w-0 flex-col gap-0.5">
                      {/* `break-all`, not `break-words`: a gradient literal is
                            one unbroken token with no spaces to break at, so
                            word-boundary wrapping would overflow the column. */}
                      <code className="break-all text-mono text-secondary">
                        {token?.value ?? "…"}
                      </code>
                      {token && !isHex(token.value) ? (
                        <span className="text-body-sm text-tertiary">
                          {describeValueKind(token.value)}
                        </span>
                      ) : null}
                    </span>
                  </span>
                </td>
              </tr>
            </Fragment>
          );
        })}
      </tbody>
    </table>
  );
}
