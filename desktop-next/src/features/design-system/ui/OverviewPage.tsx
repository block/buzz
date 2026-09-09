import { Link } from "@tanstack/react-router";
import { Badge, Button, Card } from "@buzz/ui";
import { ArrowRight, Layers, Sparkles } from "lucide-react";
import { ButtonsDemo, InputDemo, SwitchDemo } from "../catalog/forms";
import { ChartDemo } from "../catalog/display";
import { CATALOG } from "../catalog/registry";
import { PageHeader, Section } from "./primitives";
export function OverviewPage() {
  return (
    <>
      <div className="bui-inline mb-6">
        <Badge>Buzz</Badge>
        <Badge>Open-source design system</Badge>
      </div>
      <PageHeader
        title="A shared language for building together."
        intro="Quiet foundations. Expressive possibilities. A complete web component foundation for Buzz, with Inter, Lucide, and the same semantic roles from the smallest control to an agent’s response."
      />
      <div className="bui-inline mb-12">
        <Button nativeButton={false} render={<Link to="/design/components" />}>
          Explore {CATALOG.length} examples <ArrowRight aria-hidden="true" />
        </Button>
        <Button
          nativeButton={false}
          variant="outline"
          render={<Link to="/design/compositions" />}
        >
          See it in context
        </Button>
      </div>
      <div className="grid gap-6 md:grid-cols-2 mb-12">
        <Card>
          <div className="bui-inline text-caption text-secondary">
            <Layers aria-hidden="true" />
            Actions
          </div>
          <ButtonsDemo />
        </Card>
        <Card>
          <div className="bui-inline text-caption text-secondary">
            <Sparkles aria-hidden="true" />
            Everyday interactions
          </div>
          <InputDemo />
          <SwitchDemo />
        </Card>
        <Card>
          <ChartDemo />
        </Card>
        <Card>
          <p className="text-caption text-secondary">Type with a purpose</p>
          <p className="text-title">Make room for the next idea.</p>
          <p className="text-body text-secondary">
            Inter brings clarity to the interface. Every size, weight, and line
            height belongs to a named role.
          </p>
          <code className="text-code text-tertiary">
            text-title · text-body · text-caption
          </code>
          <Link to="/design/typography" className="text-label text-accent">
            Explore typography →
          </Link>
        </Card>
      </div>
      <Section
        title="One system, three layers"
        description="A change to the foundation flows through every component and every composition."
      >
        <div className="grid gap-8 md:grid-cols-3">
          {[
            [
              "01",
              "Foundations",
              "A closed vocabulary for color, type, space, corners, depth, and motion.",
              "/design/color",
            ],
            [
              "02",
              "Components",
              "Public behavior, Buzz styling, explicit variants, and accessible states.",
              "/design/components",
            ],
            [
              "03",
              "Compositions",
              "Product patterns and validated generated responses made from the same parts.",
              "/design/compositions",
            ],
          ].map(([number, title, description, to]) => (
            <div className="bui-stack" key={number}>
              <p className="text-meta text-tertiary">{number}</p>
              <h2 className="text-heading">
                <Link to={to}>{title}</Link>
              </h2>
              <p className="text-body text-secondary">{description}</p>
            </div>
          ))}
        </div>
      </Section>
      <Section title="A visual starting point, room to evolve">
        <p className="text-body text-secondary">
          Inspired by BlockUI’s visual direction and implemented for Buzz’s
          open-source architecture. Fonts and icons are public, assets are
          self-hosted, and the documentation is part of the app. Switch modes to
          inspect every example in light and dark.
        </p>
        <Link className="text-label text-accent" to="/design/open-source">
          Read the adaptation guide →
        </Link>
      </Section>
    </>
  );
}
