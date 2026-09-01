import type { RelayEvent } from "@/shared/api/types";
import type { Project, Repository } from "@/features/projects/hooks";
import {
  isValidProjectChannelId,
  MAX_PROJECT_MEMBERS,
  MAX_PROJECT_RELATED_CHANNELS,
  PROJECT_RELATED_CHANNEL_TAG,
  validateProjectEventEnvelope,
} from "@/features/projects/projectModels";
import {
  KIND_PROJECT_ANNOUNCEMENT,
  KIND_PROJECT_REVISION,
  KIND_REPO_ANNOUNCEMENT,
} from "@/shared/constants/kinds";
import type { ProjectEventTemplate } from "./projectCreation";

function repositoryDtagFromName(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

export { repositoryDtagFromName };

/**
 * Creates a project-replacement event template from a live, signed raw head
 * (fetched immediately before the mutation). Repository membership is patched,
 * and callers with a folded Project model carry its effective related channels
 * into the new base. Every other tag and the `content` field are preserved
 * verbatim, satisfying NIP-MP's extension-tag preservation rule and preventing
 * a cached UI projection from silently erasing unknown tags.
 *
 * Performs full NIP-MP envelope validation on the patched output via the shared
 * `validateProjectEventEnvelope` validator — the same checks applied by the
 * read parser — so Desktop's write path agrees with its read path on which
 * heads are valid regardless of the relay in use.
 */
function buildProjectPatchTemplate({
  liveHead,
  ownerPubkey,
  relatedChannelIds,
  repositoryAddresses,
}: {
  liveHead: RelayEvent;
  ownerPubkey: string;
  relatedChannelIds?: string[];
  repositoryAddresses: string[];
}): ProjectEventTemplate {
  const normalizedOwner = ownerPubkey.trim().toLowerCase();
  if (normalizedOwner !== liveHead.pubkey.toLowerCase()) {
    throw new Error("Only the project owner can add repositories.");
  }
  if (repositoryAddresses.length > MAX_PROJECT_MEMBERS) {
    throw new Error(
      `A project cannot contain more than ${MAX_PROJECT_MEMBERS} repositories.`,
    );
  }
  if (new Set(repositoryAddresses).size !== repositoryAddresses.length) {
    throw new Error("A project cannot contain duplicate repositories.");
  }
  if (
    repositoryAddresses.some(
      (address) => !/^30617:[0-9a-f]{64}:.+$/.test(address),
    )
  ) {
    throw new Error("Repository address is invalid.");
  }

  // Replace all existing `a` tags with the new set, preserving everything else
  // (d, name, description, buzz-channel, buzz-visibility, relay hints embedded
  // in `a` tags, and any future/unknown tags).
  const nonMemberTags = liveHead.tags.filter(
    (tag) =>
      tag[0] !== "a" &&
      (relatedChannelIds === undefined ||
        tag[0] !== PROJECT_RELATED_CHANNEL_TAG),
  );
  const existingHints = new Map<string, string>();
  for (const tag of liveHead.tags) {
    if (tag[0] === "a" && tag[1] && tag[2]) {
      existingHints.set(tag[1], tag[2]);
    }
  }
  const memberTags = repositoryAddresses.sort().map((address): string[] => {
    const hint = existingHints.get(address);
    return hint ? ["a", address, hint] : ["a", address];
  });

  const homeChannelId = liveHead.tags
    .find((tag) => tag[0] === "buzz-channel")?.[1]
    ?.toLowerCase();
  const relatedChannelTags = [
    ...new Set(
      (relatedChannelIds ?? []).map((channelId) =>
        channelId.trim().toLowerCase(),
      ),
    ),
  ]
    .filter(
      (channelId) =>
        isValidProjectChannelId(channelId) && channelId !== homeChannelId,
    )
    .slice(0, MAX_PROJECT_RELATED_CHANNELS)
    .map((channelId) => [PROJECT_RELATED_CHANNEL_TAG, channelId]);
  const patchedTags = [
    ...nonMemberTags,
    ...(relatedChannelIds === undefined ? [] : relatedChannelTags),
    ...memberTags,
  ];
  const content = liveHead.content;

  // Validate the full patched envelope against NIP-MP rules. This catches
  // nonconforming live heads (e.g., from a relay that accepted a malformed
  // event) before we sign and re-submit, and pins the write path to the same
  // spec the read parser enforces: duplicate `d`, duplicate/oversized
  // metadata, malformed member arity, and the 64-member boundary.
  validateProjectEventEnvelope(patchedTags, content);

  return {
    kind: KIND_PROJECT_ANNOUNCEMENT,
    content,
    tags: patchedTags,
  };
}

export { buildProjectPatchTemplate };

/** Reject owner replacements built from a stale base or collaborative head. */
export function assertProjectRepositoryWriteCurrent({
  project,
  liveHead,
  revisionHeads,
}: {
  project: Pick<
    Project,
    "baseRevisionId" | "effectiveRevisionId" | "projectAddress"
  >;
  liveHead: RelayEvent;
  revisionHeads: RelayEvent[];
}): void {
  const baseRevisionId = project.baseRevisionId?.toLowerCase();
  const effectiveRevisionId = project.effectiveRevisionId?.toLowerCase();
  const liveBaseId = liveHead.id.toLowerCase();
  if (!baseRevisionId || liveBaseId !== baseRevisionId) {
    throw new Error(
      "This project was updated by another session while you were working. Refresh and try again.",
    );
  }
  if (revisionHeads.length > 1) {
    throw new Error("The relay returned multiple current Project revisions.");
  }
  const revisionHead = revisionHeads[0];
  if (revisionHead) {
    const coordinate = revisionHead.tags.find((tag) => tag[0] === "a")?.[1];
    const signedBase = revisionHead.tags.find((tag) => tag[0] === "base")?.[1];
    if (
      revisionHead.kind !== KIND_PROJECT_REVISION ||
      coordinate !== project.projectAddress ||
      signedBase?.toLowerCase() !== liveBaseId
    ) {
      throw new Error(
        "The relay returned an invalid current Project revision.",
      );
    }
  }
  const liveEffectiveId = (revisionHead?.id ?? liveHead.id).toLowerCase();
  if (!effectiveRevisionId || liveEffectiveId !== effectiveRevisionId) {
    throw new Error(
      "This project's channels were updated by another session. Refresh and try again.",
    );
  }
}

export function buildRepositoryChannelBindingTemplate({
  channelId,
  ownerPubkey,
  repository,
}: {
  channelId: string;
  ownerPubkey: string;
  repository: Repository;
}): ProjectEventTemplate {
  const normalizedChannelId = channelId.trim();
  if (ownerPubkey.trim().toLowerCase() !== repository.owner.toLowerCase()) {
    throw new Error("Only the repository owner can repair its access.");
  }
  if (!isValidProjectChannelId(normalizedChannelId)) {
    throw new Error("Repository access channel is invalid.");
  }
  if (!repository.eventTags) {
    throw new Error(
      "Repository metadata is unavailable. Refresh and try again.",
    );
  }

  return {
    kind: KIND_REPO_ANNOUNCEMENT,
    content: repository.eventContent ?? repository.description,
    tags: [
      ...repository.eventTags
        .filter((tag) => tag[0] !== "buzz-channel")
        .map((tag) => [...tag]),
      ["buzz-channel", normalizedChannelId],
    ],
  };
}

export type AddedRepositoryEventTemplatesFromHead = {
  project: ProjectEventTemplate;
  repository: ProjectEventTemplate;
  repositoryAddress: string;
  repositoryDtag: string;
  /**
   * True when the live head already references the coordinate but the caller
   * indicated no repository head exists there (a dangling member from an
   * earlier partial publish). The project head is already correct — publish
   * only the repository event to heal.
   */
  resume: boolean;
};

/**
 * Builds the project-replacement + new-repository templates for `addRepo`,
 * patching the live signed project head rather than reconstructing from the
 * cached UI projection. This is the preferred path: it preserves unknown tags
 * and detects concurrent writes before they cause data loss.
 *
 * The caller is responsible for checking that `liveHead.created_at` is not
 * newer than the cached project's `createdAt` + the caller's margin (i.e., a
 * dominated-write guard) before using the result.
 */
export function buildAddedRepositoryEventTemplatesFromHead({
  accessChannelId,
  cloneUrl,
  description,
  existingRepositoryAddresses,
  liveHead,
  name,
  ownerPubkey,
  relatedChannelIds,
  repositoryHeadExists = true,
  webUrl,
}: {
  accessChannelId?: string;
  cloneUrl?: string;
  description?: string;
  existingRepositoryAddresses: string[];
  liveHead: RelayEvent;
  name: string;
  ownerPubkey: string;
  relatedChannelIds?: string[];
  /**
   * Whether a kind-30617 head already exists at the new coordinate. When the
   * live project head references the coordinate but no repository head exists
   * there, an earlier add-repository publish failed between its two events —
   * return resume templates instead of throwing so retry can heal the
   * dangling member.
   */
  repositoryHeadExists?: boolean;
  webUrl?: string;
}): AddedRepositoryEventTemplatesFromHead {
  const normalizedOwner = ownerPubkey.trim().toLowerCase();

  const normalizedName = name.trim();
  if (!normalizedName) throw new Error("Repository name is required.");
  const repositoryDtag = repositoryDtagFromName(normalizedName);
  if (!repositoryDtag) {
    throw new Error("Repository name must include letters or numbers.");
  }

  const repositoryAddress = `${KIND_REPO_ANNOUNCEMENT}:${normalizedOwner}:${repositoryDtag}`;

  // Read live membership from the fetched head (not the cached projection).
  const liveAddresses = liveHead.tags
    .filter((tag) => tag[0] === "a" && tag[1])
    .map((tag) => tag[1] as string);

  // If the repo is already in the live head with a live repository head at
  // the coordinate (race: another session added it), surface that to the
  // caller. Without a repository head the membership is a dangling member
  // from a partial publish — resume by publishing only the repository event.
  const resume =
    liveAddresses.includes(repositoryAddress) && !repositoryHeadExists;
  if (liveAddresses.includes(repositoryAddress) && repositoryHeadExists) {
    throw new Error(
      `This project already contains "${repositoryDtag}" (it was added by another session).`,
    );
  }

  // An "unavailable member" is a coordinate already in the project's address
  // list (cached projection) but absent from resolved repositories. When this
  // happens we keep the existing addresses and add nothing new.
  const isUnavailableMember =
    existingRepositoryAddresses.includes(repositoryAddress);

  const normalizedDescription = description?.trim() ?? "";
  const repositoryTags: string[][] = [
    ["d", repositoryDtag],
    ["name", normalizedName],
  ];
  const normalizedAccessChannelId = accessChannelId?.trim();
  if (!normalizedAccessChannelId) {
    throw new Error(
      "This project has no repository access channel to inherit.",
    );
  }
  if (!isValidProjectChannelId(normalizedAccessChannelId)) {
    throw new Error("Repository access channel is invalid.");
  }
  repositoryTags.push(["buzz-channel", normalizedAccessChannelId]);
  if (normalizedDescription) {
    repositoryTags.push(["description", normalizedDescription]);
  }
  const normalizedCloneUrl = cloneUrl?.trim();
  if (normalizedCloneUrl) repositoryTags.push(["clone", normalizedCloneUrl]);
  const normalizedWebUrl = webUrl?.trim();
  if (normalizedWebUrl) repositoryTags.push(["web", normalizedWebUrl]);

  // In resume mode the live head already lists the coordinate; the project
  // template is a no-op republish guard and must not double-add the address.
  const newAddresses =
    isUnavailableMember || resume
      ? [...liveAddresses]
      : [...liveAddresses, repositoryAddress];

  const projectTemplate = buildProjectPatchTemplate({
    liveHead,
    ownerPubkey,
    relatedChannelIds,
    repositoryAddresses: newAddresses,
  });

  return {
    project: projectTemplate,
    repository: {
      kind: KIND_REPO_ANNOUNCEMENT,
      content: normalizedDescription,
      tags: repositoryTags,
    },
    repositoryAddress,
    repositoryDtag,
    resume,
  };
}
