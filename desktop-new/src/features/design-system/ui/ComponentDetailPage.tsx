import { COMPONENTS } from "@/shared/ui/registry";
import { BaseUiBackingLine } from "./BaseUiBackingLine";
import { COMPONENT_SPECIMENS } from "./componentSpecimens";

export function ComponentDetailPage({ slug }: { slug: string }) {
  const component = COMPONENTS.find((candidate) => candidate.slug === slug);
  if (!component) return null;
  const Specimen = COMPONENT_SPECIMENS[component.slug];

  return (
    <>
      <header className="component-page-heading">
        <h1 className="text-title text-primary">{component.name}</h1>
        <BaseUiBackingLine
          slug={component.slug}
          behavior={component.behavior}
        />
      </header>
      {Specimen ? <Specimen /> : null}
    </>
  );
}
