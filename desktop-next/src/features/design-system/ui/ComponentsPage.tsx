import { useMemo, useState } from "react";
import { Link } from "@tanstack/react-router";
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
      <PageHeader title="Components" />
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
              <Link
                to="/design/components/$componentId"
                params={{ componentId: entry.id }}
                className="catalog-eyebrow text-meta text-tertiary hover:text-primary"
                aria-label={`${entry.category}: ${entry.name}`}
              >
                {entry.category} <span aria-hidden="true">→</span>
              </Link>
              <h2 id={`${entry.id}-title`} className="text-heading mt-1">
                {entry.name}
              </h2>
            </div>
            <div className="catalog-preview">
              <entry.Preview />
            </div>
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
