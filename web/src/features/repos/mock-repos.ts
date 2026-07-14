import type { Repo } from "./use-repos";
import type {
  BlobView,
  CommitInfo,
  ReadmeResult,
  TreeEntry,
} from "./git-client";

const now = Math.floor(Date.now() / 1000);
const people = {
  ada: "1".repeat(64),
  grace: "2".repeat(64),
  linus: "3".repeat(64),
  margaret: "4".repeat(64),
};

/** Local-only data for previewing the populated repositories state. */
export const mockRepos: Repo[] = [
  {
    id: "buzz-desktop",
    name: "buzz-desktop",
    description:
      "The desktop client for collaborating with people and agents across Buzz workspaces.",
    cloneUrls: ["https://example.com/buzz-desktop.git"],
    webUrl: null,
    channelId: null,
    owner: people.ada,
    contributors: [people.grace, people.linus],
    createdAt: now - 60 * 24,
  },
  {
    id: "agent-harness",
    name: "agent-harness",
    description:
      "Tools and shared workflows for launching and coordinating coding agents.",
    cloneUrls: ["https://example.com/agent-harness.git"],
    webUrl: null,
    channelId: null,
    owner: people.grace,
    contributors: [people.ada, people.margaret],
    createdAt: now - 60 * 60 * 3,
  },
  {
    id: "relay-infrastructure",
    name: "relay-infrastructure",
    description:
      "Infrastructure and deployment configuration for community relays.",
    cloneUrls: ["https://example.com/relay-infrastructure.git"],
    webUrl: null,
    channelId: null,
    owner: people.linus,
    contributors: [people.margaret],
    createdAt: now - 60 * 60 * 24 * 2,
  },
  {
    id: "design-system",
    name: "design-system",
    description: "Shared foundations, components, and interaction patterns.",
    cloneUrls: ["https://example.com/design-system.git"],
    webUrl: null,
    channelId: null,
    owner: people.margaret,
    contributors: [people.ada, people.grace, people.linus],
    createdAt: now - 60 * 60 * 24 * 8,
  },
];

export function getMockRepo(repoId: string): Repo | undefined {
  if (!import.meta.env.DEV) return undefined;
  return mockRepos.find((repo) => repo.id === repoId);
}

export const mockRepoTree: TreeEntry[] = [
  {
    name: "src",
    type: "tree",
    mode: "040000",
    oid: "1".repeat(40),
  },
  {
    name: "README.md",
    type: "blob",
    mode: "100644",
    oid: "2".repeat(40),
  },
  {
    name: "package.json",
    type: "blob",
    mode: "100644",
    oid: "3".repeat(40),
  },
];

export const mockRepoCommits: CommitInfo[] = [
  {
    oid: "a".repeat(40),
    message: "Polish repository browsing styles",
    author: {
      name: "Ada",
      email: "ada@example.com",
      timestamp: now - 60 * 24,
    },
  },
  {
    oid: "b".repeat(40),
    message: "Add repository empty states",
    author: {
      name: "Grace",
      email: "grace@example.com",
      timestamp: now - 60 * 60 * 5,
    },
  },
];

export const mockRepoReadme: ReadmeResult = {
  filename: "README.md",
  content:
    "# Buzz Desktop\n\nA focused workspace for people and agents to collaborate.\n\n## Getting started\n\nInstall dependencies, then start the development app.",
};

export function getMockBlob(
  repoId: string,
  filepath: string,
): BlobView | undefined {
  if (!getMockRepo(repoId)) return undefined;

  if (filepath === "README.md") {
    return {
      kind: "markdown",
      content: mockRepoReadme.content,
      sizeBytes: mockRepoReadme.content.length,
    };
  }

  if (filepath === "package.json") {
    const content = `${JSON.stringify(
      {
        name: repoId,
        private: true,
        scripts: { dev: "vite", build: "tsc && vite build" },
      },
      null,
      2,
    )}\n`;
    return { kind: "text", content, sizeBytes: content.length };
  }

  return undefined;
}
