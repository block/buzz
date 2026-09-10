import { Table } from "@buzz/ui";
import { foundationGrammar } from "../grammar/foundations";
import { typographyGrammar } from "../grammar/typography";
import { compositionGrammar } from "../grammar/composition";
import { BLOCKUI_REFERENCE } from "../grammar/types";
import { PageHeader } from "./primitives";

const sections = [
  foundationGrammar[0],
  ...typographyGrammar,
  ...foundationGrammar.slice(1),
  ...compositionGrammar,
];

/** Cited BlockUI reference guidance, separate from Buzz's active token contract. */
export function GrammarPage() {
  return (
    <article className="grammar-page">
      <PageHeader title="Logic & grammar" />
      <p className="text-caption text-secondary mb-6">
        BlockUI reference · Usage, boundaries, and composition · Source links
        require Block access
      </p>
      <nav
        aria-label="Grammar topics"
        className="flex flex-wrap gap-x-5 gap-y-2 mb-8"
      >
        {sections.map((section) => (
          <a
            key={section.id}
            href={`#${section.id}`}
            className="text-caption text-accent underline"
          >
            {section.title}
          </a>
        ))}
      </nav>
      <div className="flex flex-col gap-10">
        {sections.map((section) => (
          <section
            key={section.id}
            id={section.id}
            className="grammar-section"
            aria-labelledby={`${section.id}-heading`}
          >
            <h2
              id={`${section.id}-heading`}
              className="text-heading text-primary"
            >
              {section.title}
            </h2>
            <Table className="grammar-table mt-5 min-w-xl" tabIndex={0}>
              <caption className="bui-sr-only">{section.title}</caption>
              <thead>
                <tr>
                  <th scope="col">Choice</th>
                  <th scope="col">Use</th>
                  <th scope="col">Boundary</th>
                </tr>
              </thead>
              <tbody>
                {section.rows.map(([choice, use, boundary]) => (
                  <tr key={choice}>
                    <th scope="row">{choice}</th>
                    <td>{use}</td>
                    <td>{boundary}</td>
                  </tr>
                ))}
              </tbody>
            </Table>
            <div className="flex flex-wrap gap-x-4 gap-y-2 mt-4">
              {section.sources.map((source) => (
                <a
                  key={source}
                  href={`${BLOCKUI_REFERENCE}${source}`}
                  className="text-meta text-accent underline break-all"
                >
                  {source.split("/").at(-1)}
                </a>
              ))}
            </div>
          </section>
        ))}
      </div>
    </article>
  );
}
