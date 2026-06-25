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
    activityCount: 0,
    updatedAt: 0,
    participantPubkeys: [],
  };
  summaryByRepoAddress.set(repoAddress, summary);
  return summary;
}

export function summarizeProjectActivityEvents(events, projects) {
  const repoAddresses = new Set(projects.map((project) => project.repoAddress));
  const summaryByRepoAddress = new Map();
  const participantsByRepoAddress = new Map();

  for (const project of projects) {
    ensureSummary(summaryByRepoAddress, project.repoAddress);
    participantsByRepoAddress.set(project.repoAddress, new Set());
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

  return Object.fromEntries(summaryByRepoAddress);
}
