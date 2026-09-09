import { useMemo, useState } from "react";
import { Button, Field, Input } from "@buzz/ui";
import { CATALOG } from "../catalog/registry";
import { PageHeader } from "./primitives";
const categories = [
  "All",
  "Inputs",
  "Overlays",
  "Navigation",
  "Display & layout",
];
export function ComponentsPage() {
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState("All");
  const entries = useMemo(
    () =>
      CATALOG.filter(
        (entry) =>
          (category === "All" || entry.category === category) &&
          `${entry.name} ${entry.description}`
            .toLowerCase()
            .includes(query.toLowerCase()),
      ),
    [category, query],
  );
  return (
    <>
      <PageHeader
        title="Components"
        intro="A shared visual language, built in the open. Quiet surfaces, confident actions, and accessible behavior — all using the same Buzz roles."
      />
      <div className="bui-stack mb-10">
        <Field.Root>
          <Field.Label>Find a component</Field.Label>
          <Input
            type="search"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search buttons, dialogs, tables…"
          />
        </Field.Root>
        <div className="bui-inline">
          {categories.map((value) => (
            <Button
              key={value}
              size="sm"
              variant={category === value ? "primary" : "ghost"}
              aria-pressed={category === value}
              onClick={() => setCategory(value)}
            >
              {value}
            </Button>
          ))}
        </div>
        <p className="text-caption text-secondary" role="status">
          {entries.length} of {CATALOG.length} component examples
        </p>
      </div>
      <div className="catalog-grid">
        {entries.map((entry) => (
          <section
            key={entry.id}
            id={entry.id}
            aria-labelledby={`${entry.id}-title`}
            className="catalog-entry"
          >
            <div>
              <p className="text-meta text-tertiary">{entry.category}</p>
              <h2 id={`${entry.id}-title`} className="text-heading mt-1">
                <a href={`#${entry.id}`}>{entry.name}</a>
              </h2>
              <p className="text-caption text-secondary mt-2">
                {entry.description}
              </p>
            </div>
            <div className="catalog-preview">
              <entry.Preview />
            </div>
            <details className="catalog-source">
              <summary className="text-caption text-secondary">
                View example source
              </summary>
              <pre className="text-code">
                <code>{entry.source}</code>
              </pre>
            </details>
          </section>
        ))}
      </div>
      {entries.length === 0 && (
        <div className="bui-stack">
          <p className="text-body">No components match this search.</p>
          <Button
            variant="outline"
            onClick={() => {
              setQuery("");
              setCategory("All");
            }}
          >
            Clear filters
          </Button>
        </div>
      )}
    </>
  );
}
