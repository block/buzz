import { Link } from "@tanstack/react-router";

import {
  BASE_UI_DOCS_ROOT,
  type BaseUiPart,
  baseUiDocsUrl,
  COMPONENTS,
  type ComponentDefinition,
  resolveBaseUiBacking,
} from "@/shared/ui/registry";

import { Note, PageHeader, Section } from "./primitives";

/**
 * What each component is built on, read from the registry rather than written
 * down here — `src/shared/ui/registry.test.ts` binds those entries to the
 * imports in the component files, so a component that gains or drops a Base UI
 * part changes this page by failing that test first.
 *
 * A dash is an answer, not a gap. A header, a section, and a surface have no
 * behaviour to inherit, so wrapping them in a Base UI part would add a
 * dependency and buy nothing. The column worth watching is the third one: a
 * component with no part of its own and no inherited part is one that had to
 * author its own keyboard, focus, and assistive semantics by hand.
 */

type Backing = ReturnType<typeof resolveBaseUiBacking>;

function DocsLink({ part }: { part: BaseUiPart }) {
  return (
    <a
      href={baseUiDocsUrl(part)}
      target="_blank"
      rel="noreferrer"
      className="text-purple-12 underline decoration-1 underline-offset-2"
    >
      {part.name}
    </a>
  );
}

/**
 * The dash the question asks for, plus the word it means. A bare em dash is
 * announced as punctuation or skipped entirely, so the cell would read as empty
 * to a screen reader — which is the one reading where "no Base UI part" and "we
 * forgot to fill this in" become indistinguishable.
 */
function Dash() {
  return (
    <>
      <span aria-hidden="true" className="text-tertiary">
        —
      </span>
      <span className="sr-only">None</span>
    </>
  );
}

function OwnCell({ backing }: { backing: Backing }) {
  if (backing.own.length === 0) return <Dash />;
  return (
    <span className="flex flex-col gap-1">
      {backing.own.map((part) => (
        <code key={part.name} className="text-mono">
          <DocsLink part={part} />
        </code>
      ))}
    </span>
  );
}

function InheritedCell({ backing }: { backing: Backing }) {
  if (backing.inherited.length === 0) return <Dash />;
  return (
    <span className="flex flex-col gap-1">
      {backing.inherited.map(({ part, through }) => (
        <span key={part.name} className="flex flex-col">
          <code className="text-mono">
            <DocsLink part={part} />
          </code>
          <span className="text-body-sm text-tertiary">via {through}</span>
        </span>
      ))}
    </span>
  );
}

function BackingRow({
  component,
  backing,
}: {
  component: ComponentDefinition;
  backing: Backing;
}) {
  return (
    <tr className="border-primary border-b align-top">
      <td className="py-3 pr-4">
        {/* `break-words`: the column is a third of the width, and a long
            single-word component name overflows that third at a narrow
            viewport — without this it overlaps the next column. */}
        <Link
          to="/design/components/$component"
          params={{ component: component.slug }}
          className="break-words text-body text-primary hover:text-purple-12"
        >
          {component.name}
        </Link>
      </td>
      <td className="py-3 pr-4 text-body">
        <OwnCell backing={backing} />
      </td>
      <td className="py-3 text-body">
        <InheritedCell backing={backing} />
      </td>
    </tr>
  );
}

export function BaseUiPage() {
  const rows = COMPONENTS.map((component) => ({
    component,
    backing: resolveBaseUiBacking(component.slug),
  }));

  const backed = rows.filter(
    (row) => row.backing.own.length > 0 || row.backing.inherited.length > 0,
  );
  const native = rows.filter(
    (row) => row.backing.own.length === 0 && row.backing.inherited.length === 0,
  );

  const parts = new Set(
    rows.flatMap((row) => [
      ...row.backing.own.map((part) => part.name),
      ...row.backing.inherited.map((entry) => entry.part.name),
    ]),
  );

  return (
    <>
      <PageHeader
        title="Base UI backing"
        intro="Which components inherit behaviour from Base UI, and which author it themselves. Read from the component registry, which is bound to the imports in the component files — so a component that gains or drops a Base UI part cannot leave this page saying otherwise."
      />

      <Section
        title="Every component"
        description="The second column is what the file imports itself. The third is what reaches it through a component it composes — IconButton imports nothing from Base UI, but it renders a Buzz Button, so Base UI's Button is still underneath it."
      >
        <table className="w-full table-fixed border-collapse text-left">
          <caption className="sr-only">
            Each Buzz component, the Base UI part it imports, and the part it
            inherits through composition
          </caption>
          <colgroup>
            <col className="w-2/5" />
            <col className="w-[30%]" />
            <col className="w-[30%]" />
          </colgroup>
          <thead>
            <tr className="border-primary border-b">
              <th scope="col" className="py-2 pr-4 text-body text-tertiary">
                Component
              </th>
              <th scope="col" className="py-2 pr-4 text-body text-tertiary">
                Base UI part
              </th>
              <th scope="col" className="py-2 text-body text-tertiary">
                Inherited
              </th>
            </tr>
          </thead>
          <tbody>
            {rows.map(({ component, backing }) => (
              <BackingRow
                key={component.slug}
                component={component}
                backing={backing}
              />
            ))}
          </tbody>
        </table>
      </Section>

      <Section
        title="The count"
        description="Stated so the balance is visible rather than counted by hand each time someone asks."
      >
        <dl className="flex flex-wrap gap-x-12 gap-y-4 rounded-xl bg-neutral-2 px-5 py-4">
          <div className="flex flex-col gap-1">
            <dt className="text-body-sm text-tertiary">Backed by Base UI</dt>
            <dd className="text-body text-primary">
              {backed.length} of {rows.length}
            </dd>
          </div>
          <div className="flex flex-col gap-1">
            <dt className="text-body-sm text-tertiary">Native elements only</dt>
            <dd className="text-body text-primary">
              {native.length} of {rows.length}
            </dd>
          </div>
          <div className="flex flex-col gap-1">
            <dt className="text-body-sm text-tertiary">
              Distinct parts in use
            </dt>
            <dd className="text-body text-primary">{parts.size}</dd>
          </div>
        </dl>
      </Section>

      <Section
        title="Where a dash is the right answer"
        description="These author their own markup, and that is the decision — not an omission. A wrapper around a native header buys a dependency and no behaviour."
      >
        <div className="flex flex-col">
          {native.map(({ component }) => (
            <div
              key={component.slug}
              className="border-primary border-b py-3 last:border-b-0"
            >
              <Link
                to="/design/components/$component"
                params={{ component: component.slug }}
                className="text-body text-primary hover:text-purple-12"
              >
                {component.name}
              </Link>
              <span className="mt-0.5 block text-body-sm text-secondary">
                {component.behavior}
              </span>
            </div>
          ))}
        </div>
      </Section>

      <Note>
        A dash in the third column too means the component owns its keyboard,
        focus, and assistive semantics outright — InlineChip is the case to
        watch, because it switches between a control and an image role and has
        to get both right without help. See{" "}
        <a
          href={BASE_UI_DOCS_ROOT}
          target="_blank"
          rel="noreferrer"
          className="text-purple-12 underline"
        >
          the Base UI component index
        </a>{" "}
        for a part that could take that work over.
      </Note>
    </>
  );
}
