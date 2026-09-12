import type { ProjectIssue } from "@/features/projects/hooks";
import { issueShareLink } from "@/features/projects/lib/projectShareLinks";

/** Build the ACP execution request around an exact, copy-safe CLI operation.
 * The selected writer's own harness-injected key signs the BW transition; no
 * private key crosses the execution thread. */
export function bwStartDevelopmentRequest({
  issue,
  repositoryName,
  stream,
  writerName,
}: {
  issue: ProjectIssue;
  repositoryName: string;
  stream: string;
  writerName: string;
}): string {
  const [kind, owner, repoId] = issue.repoAddress?.split(":") ?? [];
  if (
    !/^[0-9a-f]{64}$/i.test(issue.id) ||
    kind !== "30617" ||
    !owner ||
    !/^[0-9a-f]{64}$/i.test(owner) ||
    !repoId ||
    !/^[a-zA-Z0-9._-]{1,64}$/.test(repoId)
  ) {
    throw new Error("This issue has no valid BW repository coordinate.");
  }
  const reference = `ISS-${issue.id.slice(0, 8).toUpperCase()}`;
  const link = issueShareLink(issue);
  const subject = issue.title.replace(/\s+/g, " ").trim() || reference;
  const repository = repositoryName.replace(/\s+/g, " ").trim() || repoId;
  const writer = writerName.replace(/\s+/g, " ").trim() || "Selected writer";
  const gitStream = stream.replace(/\s+/g, " ").trim();
  if (!gitStream) {
    throw new Error("The ready issue has no Git stream.");
  }
  return [
    `[EXECUTION-THREAD] ${subject}`,
    `Issue: **${repository} · ${issue.id.slice(0, 8).toLowerCase()}**`,
    `Git stream: ${gitStream}`,
    `**${writer}**: Start this issue now. You are its selected BW writer. Before editing, run this exact command so the transition is signed by your own agent identity:`,
    "```sh",
    `buzz issues start-development --issue ${issue.id.toLowerCase()} --repo-owner ${owner.toLowerCase()} --repo-id ${repoId}`,
    "```",
    link ? `Issue: ${link}` : null,
  ]
    .filter(Boolean)
    .join("\n\n");
}
