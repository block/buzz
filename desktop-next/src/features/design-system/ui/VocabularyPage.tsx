import {
  GRAMMAR,
  GRAMMAR_EXAMPLES,
  VOCABULARY,
} from "@/shared/tokens/registry";

import { PageHeader, Section } from "./primitives";

export function VocabularyPage() {
  return (
    <>
      <PageHeader title="Vocabulary" />

      <Section title="The words">
        <div className="flex flex-col gap-0 rounded-xl bg-inset px-5 py-2">
          {VOCABULARY.map((group) => (
            <div
              key={group.group}
              className="flex flex-wrap items-baseline gap-x-4 gap-y-1 border-b border-tertiary py-3 last:border-b-0"
            >
              <span className="w-24 shrink-0 text-label text-tertiary">
                {group.group}
              </span>
              <div className="flex min-w-0 flex-1 flex-wrap gap-x-2 gap-y-1">
                {group.words.map((word) => (
                  <code
                    key={word}
                    className="rounded bg-inset px-1.5 py-0.5 text-caption text-primary"
                  >
                    {word}
                  </code>
                ))}
              </div>
            </div>
          ))}
        </div>
      </Section>

      <Section title="The grammar">
        <div className="rounded-lg bg-inverse px-5 py-4">
          <code className="text-label text-on-inverse">{GRAMMAR}</code>
        </div>
        <div className="flex flex-col gap-3 sm:flex-row">
          <div className="flex-1 rounded-xl bg-success-tint px-5 py-4">
            <p className="mb-1.5 text-label text-success">Legal</p>
            {GRAMMAR_EXAMPLES.legal.map((example) => (
              <code key={example} className="block text-caption text-success">
                {example}
              </code>
            ))}
          </div>
          <div className="flex-1 rounded-xl bg-danger-tint px-5 py-4">
            <p className="mb-1.5 text-label text-danger">Illegal</p>
            {GRAMMAR_EXAMPLES.illegal.map((example) => (
              <code key={example} className="block text-caption text-danger">
                {example}
              </code>
            ))}
          </div>
        </div>
      </Section>
    </>
  );
}
