import { Table } from "@buzz/ui";
import { PageHeader, Section } from "./primitives";
const differences = [
  [
    "Visual direction",
    "Neutral surfaces, pill actions, generous corners",
    "Buzz semantic roles with independently authored CSS",
  ],
  [
    "Typography",
    "Private brand fonts",
    "Self-hosted Inter Variable and JetBrains Mono",
  ],
  ["Icons", "Private brand icon sets", "Lucide React with upstream notices"],
  [
    "Behavior",
    "Multiple exploratory component implementations",
    "Public Base UI; DayPicker and resizable panels where needed",
  ],
  [
    "Documentation",
    "Internally hosted Storybook",
    "The existing Buzz /design app, built as a static site",
  ],
  [
    "Generated UI",
    "Product-specific experiments",
    "Versioned, bounded data rendered through the same components",
  ],
];
export function OpenSourcePage() {
  return (
    <>
      <PageHeader
        title="Open by construction"
        intro="BlockUI supplies the visual starting point. Buzz owns this implementation and can evolve it independently: public dependencies, local font assets, a portable stylesheet, and one documented component contract."
      />
      <Section title="What changed">
        <Table>
          <caption className="bui-sr-only">
            Adaptation from the visual reference to Buzz
          </caption>
          <thead>
            <tr>
              <th scope="col">Area</th>
              <th scope="col">Starting point</th>
              <th scope="col">Buzz system</th>
            </tr>
          </thead>
          <tbody>
            {differences.map(([area, before, after]) => (
              <tr key={area}>
                <th scope="row">{area}</th>
                <td style={{ whiteSpace: "normal" }}>{before}</td>
                <td style={{ whiteSpace: "normal" }}>{after}</td>
              </tr>
            ))}
          </tbody>
        </Table>
      </Section>
      <Section title="The package boundary">
        <p className="text-body text-secondary">
          @buzz/design-tokens contains the role registry, light and dark values,
          typography, geometry, and motion. @buzz/ui consumes those roles. This
          catalog is a consumer of both packages; components do not import the
          catalog, router, Tauri, or a private service.
        </p>
        <pre className="text-code overflow-auto rounded-control bg-inset p-6">
          <code>{`import '@fontsource-variable/inter/index.css';\nimport { Button, Field, Input } from '@buzz/ui';\n\n<Field.Root>\n  <Field.Label>Project name</Field.Label>\n  <Input required />\n</Field.Root>\n<Button>Continue</Button>`}</code>
        </pre>
      </Section>
      <Section title="Run it anywhere">
        <p className="text-body text-secondary">
          Run the Vite catalog locally or build static files for a public host.
          A host serving the catalog needs an SPA fallback to index.html for
          nested routes. Storybook itself is open source and could be another
          consumer later; it is not a runtime dependency or a hosting
          requirement.
        </p>
        <pre className="text-code overflow-auto rounded-control bg-inset p-6">
          <code>{`pnpm install --frozen-lockfile\npnpm --filter buzz-desktop-next dev\npnpm --filter buzz-desktop-next build\npnpm --filter buzz-desktop-next preview`}</code>
        </pre>
      </Section>
      <Section title="What portability means">
        <p className="text-body text-secondary">
          This is a web and React foundation for desktop-next. It does not
          replace Flutter widgets or migrate the existing desktop client. The
          package is usable in the workspace and has a compiled distribution;
          npm publication is a separate release step. Charts currently cover
          categorical bars, and the carousel uses manual navigation. There is no
          promise of API compatibility with shadcn or internal BlockUI
          implementations.
        </p>
      </Section>
      <Section title="Provenance and licenses">
        <p className="text-body text-secondary">
          No private implementation, font, icon, screenshot, or internal
          documentation bundle is included. New Buzz code uses the repository
          license. Inter and JetBrains Mono use the SIL Open Font License;
          Lucide carries ISC and Feather-derived MIT notices; behavior
          dependencies carry their own public licenses. Full notices live in
          packages/ui/THIRD_PARTY_NOTICES.md.
        </p>
        <div className="bui-inline text-caption">
          <a
            className="text-accent underline"
            href="https://base-ui.com/react/overview/quick-start"
          >
            Base UI documentation
          </a>
          <a className="text-accent underline" href="https://lucide.dev">
            Lucide
          </a>
          <a className="text-accent underline" href="https://rsms.me/inter/">
            Inter
          </a>
        </div>
      </Section>
    </>
  );
}
