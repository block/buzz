const REPO_ID_PATTERN = /^[A-Za-z0-9._-]+$/;

export function isValidRepoId(repoId) {
  return (
    typeof repoId === "string" &&
    repoId.length > 0 &&
    repoId.length <= 64 &&
    !repoId.startsWith(".") &&
    !repoId.includes("..") &&
    REPO_ID_PATTERN.test(repoId)
  );
}

export function normalizeProjectSlug(input) {
  const slug = String(input ?? "")
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, "-")
    .replace(/-+/g, "-")
    .replace(/^[.-]+/, "")
    .replace(/[.-]+$/, "");

  return slug.slice(0, 64);
}

function assertRepoId(repoId) {
  if (!isValidRepoId(repoId)) {
    throw new Error(
      "Repository ID must be 1-64 letters, numbers, dots, underscores, or dashes, with no leading dot or double dots.",
    );
  }
}

function assertLength(name, value, max) {
  if (value && value.length > max) {
    throw new Error(`${name} must be ${max} characters or fewer.`);
  }
}

function assertUrl(name, value, allowedPrefixes, max) {
  if (!value) return;
  if (!allowedPrefixes.some((prefix) => value.startsWith(prefix))) {
    throw new Error(`${name} must start with ${allowedPrefixes.join(" or ")}.`);
  }
  assertLength(name, value, max);
}

export function getTag(event, name) {
  return event.tags.find((tag) => tag[0] === name)?.[1];
}

export function getAllTags(event, name) {
  return event.tags.filter((tag) => tag[0] === name).map((tag) => tag[1]);
}

export function getCloneUrls(event) {
  const tag = event.tags.find((candidate) => candidate[0] === "clone");
  return tag ? tag.slice(1) : [];
}

export function buildProjectCloneUrl({ relayHttpUrl, owner, repoId }) {
  assertRepoId(repoId);
  if (!/^[a-fA-F0-9]{64}$/.test(owner)) {
    throw new Error("Owner pubkey must be 64 hex characters.");
  }

  return `${relayHttpUrl.replace(/\/+$/, "")}/git/${owner.toLowerCase()}/${repoId}`;
}

export function buildRepoAnnouncementTags({
  repoId,
  name,
  description,
  cloneUrls = [],
  webUrl = null,
  relays = [],
  projectChannelId = null,
  status = "active",
  defaultBranch = "main",
  contributors = [],
}) {
  assertRepoId(repoId);
  assertLength("Project name", name, 128);
  assertLength("Description", description, 1024);
  assertLength("Status", status, 32);
  assertLength("Default branch", defaultBranch, 128);

  if (cloneUrls.length > 5) {
    throw new Error("Projects can publish at most 5 clone URLs.");
  }
  for (const url of cloneUrls) {
    assertUrl("Clone URL", url, ["http://", "https://"], 512);
  }
  assertUrl("Web URL", webUrl, ["http://", "https://"], 512);

  if (relays.length > 10) {
    throw new Error("Projects can publish at most 10 relay URLs.");
  }
  for (const relay of relays) {
    assertUrl("Relay URL", relay, ["ws://", "wss://"], 256);
  }

  const tags = [["d", repoId]];
  if (name) tags.push(["name", name]);
  if (description) tags.push(["description", description]);
  if (cloneUrls.length > 0) tags.push(["clone", ...cloneUrls]);
  if (webUrl) tags.push(["web", webUrl]);
  if (relays.length > 0) tags.push(["relays", ...relays]);
  if (projectChannelId) tags.push(["h", projectChannelId]);
  if (status) tags.push(["status", status]);
  if (defaultBranch) tags.push(["default-branch", defaultBranch]);

  for (const contributor of contributors) {
    if (/^[a-fA-F0-9]{64}$/.test(contributor)) {
      tags.push(["p", contributor.toLowerCase()]);
    }
  }

  return tags;
}

export function eventToProject(event) {
  const dtag = getTag(event, "d") ?? event.id;
  const name = getTag(event, "name") || dtag;
  const description = getTag(event, "description") || event.content || "";
  const cloneUrls = getCloneUrls(event);
  const webUrl = getTag(event, "web") ?? null;
  const projectChannelId =
    getTag(event, "h") ?? getTag(event, "project-channel") ?? null;
  const contributors = getAllTags(event, "p");

  return {
    id: `${event.pubkey}:${dtag}`,
    dtag,
    name,
    description,
    cloneUrls,
    webUrl,
    owner: event.pubkey,
    contributors,
    createdAt: event.created_at,
    projectChannelId,
    status: getTag(event, "status") ?? "active",
    defaultBranch: getTag(event, "default-branch") ?? "main",
    repoAddress: `30617:${event.pubkey}:${dtag}`,
  };
}

export function withCanonicalProjectCloneUrl(project, relayHttpUrl) {
  try {
    return {
      ...project,
      cloneUrls: [
        buildProjectCloneUrl({
          relayHttpUrl,
          owner: project.owner,
          repoId: project.dtag,
        }),
      ],
    };
  } catch {
    return {
      ...project,
      cloneUrls: project.cloneUrls.filter(
        (url) => !/github\.com/i.test(String(url)),
      ),
    };
  }
}

export function dedupProjectEvents(events) {
  const best = new Map();

  for (const event of events) {
    const dtag = getTag(event, "d") ?? "";
    const key = `${event.pubkey}:${event.kind}:${dtag}`;
    const previous = best.get(key);

    if (!previous || event.created_at > previous.created_at) {
      best.set(key, event);
    }
  }

  return [...best.values()];
}

export function projectEventsToProjects(events) {
  return dedupProjectEvents(events)
    .map(eventToProject)
    .sort((left, right) => right.createdAt - left.createdAt);
}
