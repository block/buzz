import { useEffect, useRef } from "react";
import { Link } from "@tanstack/react-router";
import { Card } from "@buzz/ui";
import type { CatalogEntry } from "../catalog/registry";
import { PageHeader } from "./primitives";

/** A focused component example, using the shared card geometry and appearance. */
export function ComponentPage({ entry }: { entry: CatalogEntry }) {
  const heading = useRef<HTMLHeadingElement>(null);
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
      <header className="mb-6">
        <h1
          ref={heading}
          tabIndex={-1}
          id="component-page-title"
          className="text-title text-primary"
        >
          {entry.name}
        </h1>
      </header>
      <Card
        className="component-preview max-w-3xl"
        aria-label={`${entry.name} example`}
      >
        <entry.Preview />
      </Card>
    </article>
  );
}

/** Keep an unknown deep link recoverable within the design-system navigation. */
export function ComponentNotFound() {
  return (
    <>
      <PageHeader title="Component not found" />
      <Link to="/design/components" className="text-label text-accent">
        Browse all components
      </Link>
    </>
  );
}
