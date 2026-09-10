import agentsSource from "../../../../AGENTS.md?raw";
import designSource from "../../../../DESIGN.md?raw";
import maintainingSource from "../../../../MAINTAINING_DESIGN_SYSTEM.md?raw";
import { MarkdownPage } from "./MarkdownPage";

const DOCUMENTS = {
  maintaining: maintainingSource,
  design: designSource,
  agents: agentsSource,
} as const;

export function SystemDocumentPage({
  document,
}: {
  document: keyof typeof DOCUMENTS;
}) {
  return <MarkdownPage source={DOCUMENTS[document]} />;
}
