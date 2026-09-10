import { ComposerStateGallery } from "./ComposerStateGallery";
import { COMPONENTS } from "@/shared/ui/registry";
import { BaseUiBackingLine } from "./BaseUiBackingLine";
import { COMPONENT_SPECIMENS } from "./componentSpecimens";

export function ComponentDetailPage({ slug }: { slug: string }) {
  const component = COMPONENTS.find((candidate) => candidate.slug === slug);
  if (!component) return null;
  const Specimen =
    component.slug === "composer"
      ? ComposerStateGallery
      : COMPONENT_SPECIMENS[component.slug];

  return (
    <>
      <header className="component-page-heading">
        <h1 className="text-title text-primary">{component.name}</h1>
        <p className="text-body text-tertiary">
          {component.slug === "composer"
            ? "Write a message, choose its destination, and explore the composer’s controls."
            : component.purpose}
          <BaseUiBackingLine slug={component.slug} />
        </p>
      </header>
      {Specimen ? <Specimen /> : null}
    </>
  );
}
