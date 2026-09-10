import { Table } from "@buzz/ui";
import { useEffect } from "react";
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
  useEffect(() => {
    const revealTopic = () => {
      const topic = document.getElementById(window.location.hash.slice(1));
      if (topic instanceof HTMLDetailsElement) {
        topic.open = true;
        topic.scrollIntoView({ block: "start" });
      }
    };
    revealTopic();
    window.addEventListener("hashchange", revealTopic);
    return () => window.removeEventListener("hashchange", revealTopic);
  }, []);

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
            onClick={() => {
              const topic = document.getElementById(section.id);
              if (topic instanceof HTMLDetailsElement) topic.open = true;
            }}
            className="text-caption text-accent underline"
          >
            {section.title}
          </a>
        ))}
      </nav>
      <div className="flex flex-col gap-4">
        {sections.map((section) => (
          <details
            key={section.id}
            id={section.id}
            className="grammar-section rounded-control bg-inset p-4"
            open={section.id === "role-resolution"}
          >
            <summary className="cursor-pointer text-subheading text-primary">
              {section.title}
            </summary>
            <div className="hidden sm:block">
              <Table className="grammar-table mt-5">
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
            </div>
            <dl className="sm:hidden mt-4 flex flex-col gap-5">
              {section.rows.map(([choice, use, boundary]) => (
                <div key={choice} className="flex flex-col gap-2">
                  <dt className="text-label text-primary break-words">
                    {choice}
                  </dt>
                  <dd className="text-body text-primary">{use}</dd>
                  <dd className="text-caption text-secondary">{boundary}</dd>
                </div>
              ))}
            </dl>
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
          </details>
        ))}
      </div>
    </article>
  );
}
