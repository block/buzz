import { Markdown } from "@/shared/ui/markdown";

import { AgentDefinitionMetadata } from "./AgentDefinitionMetadata";

const AGENT_INSTRUCTION_MARKDOWN_CLASS_NAME = [
  "mt-3 w-full min-w-0 max-w-full overflow-x-hidden leading-6 text-muted-foreground [&>*]:min-w-0 [&>*]:max-w-full [&_.code-block-lines]:min-w-0 [&_.code-block-lines]:max-w-full [&_.code-block-lines]:whitespace-pre-wrap [&_.code-block-lines]:[overflow-wrap:anywhere] [&_.inline-code-chip]:max-w-full [&_.inline-code-chip]:whitespace-pre-wrap [&_.inline-code-chip]:[overflow-wrap:anywhere] [&_blockquote]:!text-muted-foreground [&_code]:!text-muted-foreground [&_li]:text-muted-foreground [&_ol]:text-muted-foreground [&_p]:text-muted-foreground [&_strong]:text-muted-foreground [&_td]:text-muted-foreground [&_ul]:text-muted-foreground",
  "[&>h1]:!text-sm [&>h1]:!font-semibold [&>h1]:!leading-6 [&>h1]:!tracking-normal [&>h1]:!text-foreground",
  "[&>h2]:!text-sm [&>h2]:!font-semibold [&>h2]:!leading-6 [&>h2]:!tracking-normal [&>h2]:!text-foreground",
  "[&>h3]:!text-sm [&>h3]:!font-semibold [&>h3]:!leading-6 [&>h3]:!tracking-normal [&>h3]:!text-foreground",
  "[&>h4]:!text-sm [&>h4]:!font-semibold [&>h4]:!leading-6 [&>h4]:!tracking-normal [&>h4]:!text-foreground",
  "[&>h5]:!text-sm [&>h5]:!font-semibold [&>h5]:!leading-6 [&>h5]:!tracking-normal [&>h5]:!text-foreground",
  "[&>h6]:!text-sm [&>h6]:!font-semibold [&>h6]:!leading-6 [&>h6]:!tracking-normal [&>h6]:!text-foreground",
].join(" ");

const SUMMARY_MAX_LENGTH = 160;

export function DefinitionMarkdown({ content }: { content: string }) {
  return (
    <Markdown
      className={AGENT_INSTRUCTION_MARKDOWN_CLASS_NAME}
      content={content}
      interactive={false}
    />
  );
}

function markdownToPlainText(value: string) {
  return value
    .replace(/```(?:[^\n]*)\n?([\s\S]*?)```/g, "$1")
    .split("\n")
    .map((line) =>
      line
        .trim()
        .replace(/^#{1,6}\s+/, "")
        .replace(/^>\s?/, "")
        .replace(/^[-*+]\s+\[[ xX]\]\s+/, "")
        .replace(/^[-*+]\s+/, "")
        .replace(/^\d+\.\s+/, "")
        .replace(/!\[([^\]]*)\]\([^)]+\)/g, "$1")
        .replace(/\[([^\]]+)\]\([^)]+\)/g, "$1")
        .replace(/`([^`]+)`/g, "$1")
        .replace(/(\*\*|__)(.*?)\1/g, "$2")
        .replace(/(\*|_)(.*?)\1/g, "$2")
        .replace(/~~(.*?)~~/g, "$1")
        .replace(/<[^>]*>/g, "")
        .trim(),
    )
    .filter(Boolean)
    .join(" ")
    .replace(/\s+/g, " ")
    .trim();
}

function firstSentence(value: string) {
  const sentenceEnd = value.search(/[.!?](?:\s|$)/);
  return sentenceEnd >= 0 ? value.slice(0, sentenceEnd + 1) : value;
}

function truncateSummary(value: string) {
  if (value.length <= SUMMARY_MAX_LENGTH) {
    return value;
  }

  const candidate = value.slice(0, SUMMARY_MAX_LENGTH - 1);
  const lastSpace = candidate.lastIndexOf(" ");
  const end = lastSpace > SUMMARY_MAX_LENGTH / 2 ? lastSpace : candidate.length;
  return `${candidate.slice(0, end).trimEnd()}…`;
}

/**
 * Produces a concise, plain-text row summary from portable agent metadata.
 * The profile summary wins; agent instructions are only a fallback.
 */
export function getAgentInstructionSummary(
  summary: string | null | undefined,
  systemPrompt: string | null | undefined,
) {
  const source =
    markdownToPlainText(summary ?? "") ||
    markdownToPlainText(systemPrompt ?? "");
  return source ? truncateSummary(firstSentence(source)) : null;
}

/**
 * Shared agent-definition presentation used by Discover Agents and snapshot
 * previews so metadata and instruction markdown follow one visual path.
 */
export function AgentDefinitionDetails({
  isBuiltIn,
  model,
  runtime,
  systemPrompt,
}: {
  isBuiltIn: boolean;
  model: string | null;
  runtime: string | null;
  systemPrompt: string;
}) {
  return (
    <>
      <AgentDefinitionMetadata
        isBuiltIn={isBuiltIn}
        model={model}
        runtime={runtime}
      />

      <div className="min-w-0 max-w-full pt-3">
        <p className="text-base font-semibold text-foreground">
          Agent instruction
        </p>
        <DefinitionMarkdown
          content={systemPrompt || "No agent instruction included."}
        />
      </div>
    </>
  );
}
