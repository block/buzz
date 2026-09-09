import { useEffect, useRef, useState } from "react";
import { Link, useNavigate } from "@tanstack/react-router";
import { Button, Select } from "@buzz/ui";
import { Check, ChevronDown, RotateCcw } from "lucide-react";
import { CATALOG, type CatalogEntry } from "../catalog/registry";
import { STATE_GUIDES } from "../catalog/state-guides";
import { FieldStates, TextareaStates } from "../catalog/states/fields";
import {
  CheckboxStates,
  SwitchStates,
  TabsStates,
} from "../catalog/states/selection";
import { MeterStates, ProgressStates } from "../catalog/states/measurements";
import { PageHeader } from "./primitives";

const STATE_EXAMPLES = {
  input: FieldStates,
  textarea: TextareaStates,
  checkbox: CheckboxStates,
  switch: SwitchStates,
  tabs: TabsStates,
  progress: ProgressStates,
  meter: MeterStates,
};
const choices = CATALOG.map((entry) => ({
  value: entry.id,
  label: entry.name,
}));

function ComponentPicker({ entry }: { entry: CatalogEntry }) {
  const navigate = useNavigate();
  return (
    <Select.Root
      items={choices}
      value={entry.id}
      onValueChange={(value) => {
        if (value)
          void navigate({
            to: "/design/components/$componentId",
            params: { componentId: value },
          });
      }}
    >
      <Select.Trigger aria-label="Jump to component">
        <Select.Value />
        <Select.Icon>
          <ChevronDown aria-hidden="true" className="bui-icon" />
        </Select.Icon>
      </Select.Trigger>
      <Select.Portal>
        <Select.Positioner sideOffset={8}>
          <Select.Popup>
            <Select.List>
              {choices.map((choice) => (
                <Select.Item key={choice.value} value={choice.value}>
                  <Select.ItemText>{choice.label}</Select.ItemText>
                  <Select.ItemIndicator>
                    <Check aria-hidden="true" className="bui-icon" />
                  </Select.ItemIndicator>
                </Select.Item>
              ))}
            </Select.List>
          </Select.Popup>
        </Select.Positioner>
      </Select.Portal>
    </Select.Root>
  );
}

/** One component's live behavior, visible states, and navigation through the catalog. */
export function ComponentPage({ entry }: { entry: CatalogEntry }) {
  const [revision, setRevision] = useState(0);
  const heading = useRef<HTMLHeadingElement>(null);
  const index = CATALOG.findIndex((candidate) => candidate.id === entry.id);
  const previous = CATALOG[index - 1];
  const next = CATALOG[index + 1];
  const StateExamples = STATE_EXAMPLES[entry.id as keyof typeof STATE_EXAMPLES];

  useEffect(() => {
    const originalTitle = document.title;
    document.title = `${entry.name} — Buzz Design System`;
    heading.current?.focus({ preventScroll: true });
    return () => {
      document.title = originalTitle;
    };
  }, [entry.name]);

  return (
    <article className="component-page" aria-labelledby="component-page-title">
      <div className="bui-inline justify-between mb-8">
        <Link
          to="/design/components"
          hash={entry.id}
          className="text-label text-accent"
        >
          ← All components
        </Link>
        <ComponentPicker entry={entry} />
      </div>
      <header className="mb-10 bui-stack">
        <p className="text-meta text-tertiary">{entry.category}</p>
        <h1
          ref={heading}
          tabIndex={-1}
          id="component-page-title"
          className="text-title text-primary"
        >
          {entry.name}
        </h1>
        <p className="text-body-lg text-secondary max-w-2xl">
          {entry.description}
        </p>
      </header>
      <div className="component-workspace">
        <section aria-labelledby="playground-title" className="min-w-0">
          <div className="bui-inline justify-between mb-4">
            <h2 id="playground-title" className="text-heading">
              Playground
            </h2>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => setRevision((value) => value + 1)}
            >
              <RotateCcw aria-hidden="true" /> Reset example
            </Button>
          </div>
          <div className="component-playground" key={`${entry.id}-${revision}`}>
            <entry.Preview />
          </div>
        </section>
        <aside aria-labelledby="states-title" className="min-w-0">
          <h2 id="states-title" className="text-subheading mb-4">
            States to explore
          </h2>
          <ul className="bui-stack text-body text-secondary list-disc pl-5">
            {STATE_GUIDES[entry.id].map((state) => (
              <li key={state}>{state}</li>
            ))}
          </ul>
          <p className="text-caption text-tertiary mt-6">
            Use the theme and density controls to inspect the same component in
            each setting.
          </p>
        </aside>
      </div>
      {StateExamples && (
        <section aria-labelledby="state-examples-title" className="my-10">
          <h2 id="state-examples-title" className="text-heading mb-6">
            State examples
          </h2>
          <div key={`${entry.id}-states-${revision}`}>
            <StateExamples />
          </div>
        </section>
      )}
      <details className="catalog-source mt-10">
        <summary className="text-caption text-secondary">
          View example module
        </summary>
        <pre className="text-code">
          <code>{entry.source}</code>
        </pre>
      </details>
      <nav
        aria-label="Browse components"
        className="bui-inline justify-between mt-10"
      >
        {previous ? (
          <Link
            className="text-label text-accent"
            to="/design/components/$componentId"
            params={{ componentId: previous.id }}
          >
            ← Previous: {previous.name}
          </Link>
        ) : (
          <span />
        )}
        {next && (
          <Link
            className="text-label text-accent"
            to="/design/components/$componentId"
            params={{ componentId: next.id }}
          >
            Next: {next.name} →
          </Link>
        )}
      </nav>
    </article>
  );
}

/** Keep an unknown deep link recoverable within the design-system navigation. */
export function ComponentNotFound() {
  return (
    <>
      <PageHeader
        title="Component not found"
        intro="This component page does not exist. Explore the catalog to find an example."
      />
      <Link to="/design/components" className="text-label text-accent">
        Browse all components
      </Link>
    </>
  );
}
