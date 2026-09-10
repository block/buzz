import { Link } from "@tanstack/react-router";
import { COMPONENTS } from "@/shared/ui/registry";
import { COMPONENT_SPECIMENS } from "./componentSpecimens";

export function ComponentsPage() {
  const topLevelComponents = COMPONENTS.filter(
    (component) =>
      component.collection === "components" && component.parent === undefined,
  );

  return (
    <>
      <header className="component-page-heading">
        <h1 className="text-title text-primary">Components</h1>
      </header>
      <div className="component-overview-grid">
        {topLevelComponents.map((component) => {
          const Specimen = COMPONENT_SPECIMENS[component.slug];
          const children = COMPONENTS.filter(
            (candidate) => candidate.parent === component.slug,
          );
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
              {children.length > 0 ? (
                <nav aria-label={`${component.name} components`}>
                  {children.map((child) => (
                    <Link
                      key={child.slug}
                      to="/design/components/$component"
                      params={{ component: child.slug }}
                      className="text-body-sm text-secondary hover:text-primary"
                    >
                      {child.name}
                    </Link>
                  ))}
                </nav>
              ) : null}
            </article>
          );
        })}
      </div>
    </>
  );
}
