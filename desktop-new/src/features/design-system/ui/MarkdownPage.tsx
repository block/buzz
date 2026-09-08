import type { ReactNode } from "react";

function inline(text: string): ReactNode[] {
  let occurrence = 0;
  return text.split(/(`[^`]+`|\*\*[^*]+\*\*)/).map((part) => {
    if (part.startsWith("`") && part.endsWith("`")) {
      occurrence += 1;
      return <code key={`${part}-${occurrence}`}>{part.slice(1, -1)}</code>;
    }
    if (part.startsWith("**") && part.endsWith("**")) {
      occurrence += 1;
      return <strong key={`${part}-${occurrence}`}>{part.slice(2, -2)}</strong>;
    }
    return part;
  });
}

/** A deliberately small reader for the three maintained, human-facing docs.
 * These documents are prose first; this is not a general Markdown product
 * renderer. Unsupported constructs remain readable text rather than creating a
 * second documentation format that can drift from the source file. */
export function MarkdownPage({ source }: { source: string }) {
  const lines = source.split("\n");
  const content: ReactNode[] = [];
  let paragraph: string[] = [];
  let list: string[] = [];

  const flushParagraph = () => {
    if (paragraph.length) {
      content.push(
        <p
          className="design-doc-paragraph text-body text-secondary"
          key={`p-${content.length}`}
        >
          {inline(paragraph.join(" "))}
        </p>,
      );
      paragraph = [];
    }
  };
  const flushList = () => {
    if (list.length) {
      content.push(
        <ul className="design-doc-list" key={`l-${content.length}`}>
          {list.map((item) => (
            <li className="text-body text-secondary" key={item}>
              {inline(item)}
            </li>
          ))}
        </ul>,
      );
      list = [];
    }
  };

  for (const line of lines) {
    if (line.startsWith("# ")) {
      flushParagraph();
      flushList();
      content.push(
        <header className="design-doc-title" key={`h1-${content.length}`}>
          <h1 className="text-title text-primary">{line.slice(2)}</h1>
        </header>,
      );
    } else if (line.startsWith("## ")) {
      flushParagraph();
      flushList();
      content.push(
        <h2
          className="design-doc-heading text-heading text-primary"
          key={`h2-${content.length}`}
        >
          {line.slice(3)}
        </h2>,
      );
    } else if (line.startsWith("- ")) {
      flushParagraph();
      list.push(line.slice(2));
    } else if (line.trim() === "") {
      flushParagraph();
      flushList();
    } else if (!line.startsWith("|") && !line.startsWith("```")) {
      paragraph.push(line.trim());
    }
  }
  flushParagraph();
  flushList();

  return <article className="design-doc">{content}</article>;
}
