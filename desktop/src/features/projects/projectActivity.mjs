function getAllTags(event, name) {
  return event.tags.filter((tag) => tag[0] === name).map((tag) => tag[1]);
}

function getTag(event, name) {
  return event.tags.find((tag) => tag[0] === name)?.[1];
}

function normalizePubkey(pubkey) {
  return /^[a-fA-F0-9]{64}$/.test(pubkey) ? pubkey.toLowerCase() : null;
}

function ensureSummary(summaryByRepoAddress, repoAddress) {
  const existing = summaryByRepoAddress.get(repoAddress);
  if (existing) return existing;

  const summary = {
    repoAddress,
    issueCount: 0,
    prCount: 0,
    commitCount: 0,
    activityCount: 0,
    updatedAt: 0,
    participantPubkeys: [],
    latestCommit: null,
  };
  summaryByRepoAddress.set(repoAddress, summary);
  return summary;
}

export function summarizeProjectActivityEvents(events, projects) {
  const repoAddresses = new Set(projects.map((project) => project.repoAddress));
  const summaryByRepoAddress = new Map();
  const participantsByRepoAddress = new Map();
  const commitsByRepoAddress = new Map();

  for (const project of projects) {
    ensureSummary(summaryByRepoAddress, project.repoAddress);
    participantsByRepoAddress.set(project.repoAddress, new Set());
    commitsByRepoAddress.set(project.repoAddress, new Set());
  }

  for (const event of events) {
    const repoAddress = getTag(event, "a");
    if (!repoAddress || !repoAddresses.has(repoAddress)) {
      continue;
    }

    const summary = ensureSummary(summaryByRepoAddress, repoAddress);
    const participants =
      participantsByRepoAddress.get(repoAddress) ?? new Set();
    participantsByRepoAddress.set(repoAddress, participants);

    summary.activityCount += 1;
    summary.updatedAt = Math.max(summary.updatedAt, event.created_at);

    if (event.kind === 1621) {
      summary.issueCount += 1;
    }
    if (event.kind === 1618) {
      summary.prCount += 1;
    }

    // Patches (1617), pull requests (1618), and PR updates (1619) carry
    // pushed commits — track distinct hashes for the commit count and the
    // most recent one for "latest commit" labels.
    if (event.kind === 1617 || event.kind === 1618 || event.kind === 1619) {
      const commits = commitsByRepoAddress.get(repoAddress) ?? new Set();
      commitsByRepoAddress.set(repoAddress, commits);
      for (const hash of [
        ...getAllTags(event, "c"),
        ...getAllTags(event, "commit"),
      ]) {
        if (hash) {
          commits.add(hash);
        }
      }

      const commit = getTag(event, "c") ?? getTag(event, "commit");
      if (
        commit &&
        event.created_at >= (summary.latestCommit?.createdAt ?? 0)
      ) {
        summary.latestCommit = {
          author: normalizePubkey(event.pubkey),
          commit,
          createdAt: event.created_at,
          title: getTag(event, "subject") || event.content.split("\n")[0] || "",
        };
      }
    }

    const author = normalizePubkey(event.pubkey);
    if (author) {
      participants.add(author);
    }

    for (const pubkey of getAllTags(event, "p")) {
      const normalized = normalizePubkey(pubkey);
      if (normalized) {
        participants.add(normalized);
      }
    }
  }

  for (const [repoAddress, participants] of participantsByRepoAddress) {
    const summary = ensureSummary(summaryByRepoAddress, repoAddress);
    summary.participantPubkeys = [...participants].sort();
  }

  for (const [repoAddress, commits] of commitsByRepoAddress) {
    const summary = ensureSummary(summaryByRepoAddress, repoAddress);
    summary.commitCount = commits.size;
  }

  return Object.fromEntries(summaryByRepoAddress);
}
