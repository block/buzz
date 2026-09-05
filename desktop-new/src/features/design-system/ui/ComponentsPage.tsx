import { Link } from "@tanstack/react-router";
import { COMPONENTS } from "@/shared/ui/registry";
import { COMPONENT_SPECIMENS } from "./componentSpecimens";

export function ComponentsPage() {
  return (
    <>
      <header className="component-page-heading">
        <h1 className="text-title text-primary">Components</h1>
      </header>
      <div className="component-overview-grid">
        {COMPONENTS.map((component) => {
          const Specimen = COMPONENT_SPECIMENS[component.slug];
          return (
            <article key={component.slug} className="component-overview-item">
              <div className="component-overview-preview">
                {Specimen ? <Specimen /> : null}
              </div>
              <Link
                to="/design/components/$component"
                params={{ component: component.slug }}
                className="text-body text-primary"
              >
                {component.name}
              </Link>
            </article>
          );
        })}
      </div>
    </>
  );
}
