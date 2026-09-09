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
      <PageHeader title="Open by construction" />
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
          @buzz/design-tokens defines the tokens; @buzz/ui provides the
          components.
        </p>
        <pre className="text-code overflow-auto rounded-control bg-inset p-6">
          <code>{`import '@fontsource-variable/inter/index.css';\nimport { Button, Field, Input } from '@buzz/ui';\n\n<Field.Root>\n  <Field.Label>Project name</Field.Label>\n  <Input required />\n</Field.Root>\n<Button>Continue</Button>`}</code>
        </pre>
      </Section>
      <Section title="Run it anywhere">
        <p className="text-body text-secondary">
          Static hosts need an index.html fallback for nested routes.
        </p>
        <pre className="text-code overflow-auto rounded-control bg-inset p-6">
          <code>{`pnpm install --frozen-lockfile\npnpm --filter buzz-desktop-next dev\npnpm --filter buzz-desktop-next build\npnpm --filter buzz-desktop-next preview`}</code>
        </pre>
      </Section>
      <Section title="What portability means">
        <p className="text-body text-secondary">
          React components for desktop-next, available as workspace packages and
          compiled files.
        </p>
      </Section>
      <Section title="Provenance and licenses">
        <p className="text-body text-secondary">
          Repository license for Buzz code; SIL OFL for fonts; ISC and MIT
          notices for Lucide. See packages/ui/THIRD_PARTY_NOTICES.md for
          dependency licenses.
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
