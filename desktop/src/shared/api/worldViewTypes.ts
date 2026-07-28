import type { WorldViewTileSurfaceProps } from "@shivai.space/world-view-react";
import { z } from "zod";

export type WorldViewReference =
  | {
      kind: "local-world-mirror-latest";
      origin: string;
      mirrorId: string;
    }
  | {
      kind: "hosted-world-latest";
      origin: string;
      hostedWorldId: string;
    }
  | {
      kind: "hosted-world-view-export";
      origin: string;
      shareToken: string;
    }
  | {
      kind: "hosted-world-live-view-share";
      origin: string;
      shareToken: string;
    };

export type WorldViewAuthority =
  | (Extract<WorldViewReference, { kind: "local-world-mirror-latest" }> & {
      sourceRoot: string;
    })
  | Extract<WorldViewReference, { kind: "hosted-world-latest" }>;

export type WorldViewAuthorityListResult = {
  authorities: WorldViewAuthority[];
  trustedOrigins: string[];
};

export type SetWorldOriginTrustInput = {
  origin: string;
};

export type SetWorldOriginTrustResult = {
  origin: string;
  trusted: boolean;
  changed: boolean;
};
export type WorldMutationAuthority =
  | Extract<WorldViewReference, { kind: "local-world-mirror-latest" }>
  | Extract<WorldViewReference, { kind: "hosted-world-latest" }>;

export type WorldViewMutationDelegation = {
  channelId: string;
  declaredScope: WorldViewBindingScope;
  bindingId: string;
  bindingRevisionEventId: string;
  authority: WorldMutationAuthority;
};

export type WorldViewMutationDelegationListResult = {
  delegations: WorldViewMutationDelegation[];
};

export type AuthorizeWorldViewMutationInput = {
  channelId: string;
  declaredScope: WorldViewBindingScope;
  bindingId: string;
  bindingRevisionEventId: string;
  authority: WorldMutationAuthority;
};

export type AuthorizeWorldViewMutationResult = {
  delegation: WorldViewMutationDelegation;
};

export type RevokeWorldViewMutationInput = {
  channelId: string;
  declaredScope: WorldViewBindingScope;
  bindingId: string;
};

export type RevokeWorldViewMutationResult = {
  revoked: boolean;
};

export type ConnectLocalWorldAuthorityInput = {
  sourceRoot: string;
};

export type ConnectLocalWorldAuthorityResult = {
  authority: {
    origin: string;
    mirrorId: string;
    sourceRoot: string;
  };
  worldRef: Extract<WorldViewReference, { kind: "local-world-mirror-latest" }>;
};

export type WorldViewCatalogEntry = {
  name: string;
  qualifiedName: string;
  realm: {
    name: string;
    qualifiedName: string;
  };
};

export type WorldViewCatalog = {
  formatVersion: 1;
  revision: string;
  worldQualifiedName: string;
  views: WorldViewCatalogEntry[];
};

export type RegisterHostedWorldAuthorityInput = {
  origin: string;
  credential: string;
};

export type RegisterHostedWorldAuthorityResult = {
  authority: {
    origin: string;
    hostedWorldId: string;
    credentialFile: string;
  };
  revision: string;
  worldRef: Extract<WorldViewReference, { kind: "hosted-world-latest" }>;
};

export type PublishedHostedWorldLiveViewShare = {
  hostedWorldId: string;
  sourceRevision: string;
  packageRevision: string;
  realmQualifiedName: string;
  viewQualifiedName: string;
  shareToken: string;
  shareUrlPath: string;
  title: string;
};

export type WorldViewBindingScope =
  | { kind: "channel" }
  | { kind: "thread"; threadRootEventId: string };

export type WorldViewBinding = {
  id: string;
  label?: string;
  reference: WorldViewReference;
  realmQualifiedName: string;
  viewQualifiedName: string;
  displayMode: "graph" | "tasks";
};

export type WorldViewBindingsDocument = {
  version: 4;
  scope: WorldViewBindingScope;
  bindings: WorldViewBinding[];
};

export type WorldViewBindingsResponse = {
  document: WorldViewBindingsDocument;
  revisionEventId: string | null;
  updatedAt: number | null;
  author: string | null;
  nextReadCommand: string;
};

export type EffectiveWorldViewBinding = {
  binding: WorldViewBinding;
  declaredScope: WorldViewBindingScope;
  bindingRevisionEventId: string;
};

export type EffectiveWorldViewBindings = {
  effectiveScope: WorldViewBindingScope;
  bindings: EffectiveWorldViewBinding[];
  channelRevisionEventId: string | null;
  threadRevisionEventId: string | null;
  nextReadCommands: string[];
};

export type SetWorldViewBindingsInput = {
  channelId: string;
  expectedRevisionEventId: string | null;
  document: WorldViewBindingsDocument;
};

export type SetWorldViewBindingsResult = {
  ok: boolean;
  revisionEventId: string;
  nextReadCommand: string;
};

export type WorldViewResolutionRequest = {
  channelId: string;
  binding: WorldViewBinding;
  declaredScope: WorldViewBindingScope;
  effectiveScope: WorldViewBindingScope;
  bindingRevisionEventId: string;
};

const worldViewBindingScopeSchema = z.discriminatedUnion("kind", [
  z.strictObject({ kind: z.literal("channel") }),
  z.strictObject({
    kind: z.literal("thread"),
    threadRootEventId: z.string().regex(/^[0-9a-f]{64}$/),
  }),
]);

const localWorldMirrorLatestReferenceSchema = z.strictObject({
  kind: z.literal("local-world-mirror-latest"),
  origin: z.url(),
  mirrorId: z.string().min(1),
});

const hostedWorldLatestReferenceSchema = z.strictObject({
  kind: z.literal("hosted-world-latest"),
  origin: z.url(),
  hostedWorldId: z.string().min(1),
});

const hostedWorldViewExportReferenceSchema = z.strictObject({
  kind: z.literal("hosted-world-view-export"),
  origin: z.url(),
  shareToken: z.string().min(1),
});

const hostedWorldLiveViewShareReferenceSchema = z.strictObject({
  kind: z.literal("hosted-world-live-view-share"),
  origin: z.url(),
  shareToken: z.string().min(1),
});

const worldViewReferenceSchema = z.discriminatedUnion("kind", [
  localWorldMirrorLatestReferenceSchema,
  hostedWorldLatestReferenceSchema,
  hostedWorldViewExportReferenceSchema,
  hostedWorldLiveViewShareReferenceSchema,
]);

const worldViewAuthorityListResultSchema = z.strictObject({
  authorities: z.array(
    z.discriminatedUnion("kind", [
      localWorldMirrorLatestReferenceSchema.extend({
        sourceRoot: z.string().min(1),
      }),
      hostedWorldLatestReferenceSchema,
    ]),
  ),
  trustedOrigins: z.array(z.url()),
});

const setWorldOriginTrustResultSchema = z.strictObject({
  origin: z.url(),
  trusted: z.boolean(),
  changed: z.boolean(),
});
const worldMutationAuthoritySchema = z.discriminatedUnion("kind", [
  localWorldMirrorLatestReferenceSchema,
  hostedWorldLatestReferenceSchema,
]);

const worldViewMutationDelegationSchema = z.strictObject({
  channelId: z.uuid(),
  declaredScope: worldViewBindingScopeSchema,
  bindingId: z.uuid(),
  bindingRevisionEventId: z.string().regex(/^[0-9a-f]{64}$/),
  authority: worldMutationAuthoritySchema,
});

const worldViewMutationDelegationListResultSchema = z.strictObject({
  delegations: z.array(worldViewMutationDelegationSchema),
});

const authorizeWorldViewMutationResultSchema = z.strictObject({
  delegation: worldViewMutationDelegationSchema,
});

const revokeWorldViewMutationResultSchema = z.strictObject({
  revoked: z.boolean(),
});

const connectLocalWorldAuthorityResultSchema = z.strictObject({
  authority: z.strictObject({
    origin: z.url(),
    mirrorId: z.string().min(1),
    sourceRoot: z.string().min(1),
  }),
  worldRef: localWorldMirrorLatestReferenceSchema,
});

const worldViewCatalogSchema = z.strictObject({
  formatVersion: z.literal(1),
  revision: z.string().min(1),
  worldQualifiedName: z.string().min(1),
  views: z.array(
    z.strictObject({
      name: z.string().min(1),
      qualifiedName: z.string().min(1),
      realm: z.strictObject({
        name: z.string().min(1),
        qualifiedName: z.string().min(1),
      }),
    }),
  ),
});

const publishedHostedWorldLiveViewShareSchema = z.strictObject({
  hostedWorldId: z.string().min(1),
  sourceRevision: z.string().min(1),
  packageRevision: z.string().min(1),
  realmQualifiedName: z.string().min(1),
  viewQualifiedName: z.string().min(1),
  shareToken: z.string().min(1),
  shareUrlPath: z.string().min(1),
  title: z.string().min(1),
});

const worldViewBindingSchema = z.strictObject({
  id: z.uuid(),
  label: z.string().min(1).optional(),
  reference: worldViewReferenceSchema,
  realmQualifiedName: z.string().min(1),
  viewQualifiedName: z.string().min(1),
  displayMode: z.enum(["graph", "tasks"]),
});

const revisionEventIdSchema = z.string().regex(/^[0-9a-f]{64}$/);

const effectiveWorldViewBindingsSchema = z.strictObject({
  effectiveScope: worldViewBindingScopeSchema,
  bindings: z.array(
    z.strictObject({
      binding: worldViewBindingSchema,
      declaredScope: worldViewBindingScopeSchema,
      bindingRevisionEventId: revisionEventIdSchema,
    }),
  ),
  channelRevisionEventId: revisionEventIdSchema.nullable(),
  threadRevisionEventId: revisionEventIdSchema.nullable(),
  nextReadCommands: z.array(z.string().min(1)),
});

const worldViewEntitySchema = z.strictObject({
  name: z.string(),
  qualifiedName: z.string(),
});

const worldViewGraphNodeSchema = z.looseObject({
  id: z.string(),
  clusterId: z.string().optional(),
  label: z.string(),
  preferenceQualifiedName: z.string(),
  status: z.enum(["default", "leaf", "ready", "done", "goal", "focus"]),
  targetState: z.enum(["actionable", "implementing", "satisfied"]).nullable(),
  isReady: z.boolean().optional(),
  isLeaf: z.boolean(),
  signalCaseNames: z.array(z.string()).optional(),
  fillHex: z.string(),
  borderHex: z.string(),
  textHex: z.string(),
  deemphasis: z.enum(["fade", "ghost", "hide"]).nullable(),
  effect: z
    .enum(["blur", "dreamy", "focused", "glow", "prismatic", "clear"])
    .nullable(),
  position: z.strictObject({ x: z.number(), y: z.number() }),
  size: z.strictObject({ width: z.number(), height: z.number() }),
});

const worldViewGraphEdgeSchema = z.looseObject({
  deemphasis: z.enum(["fade", "ghost", "hide"]).nullable(),
  id: z.string(),
  lineHex: z.string(),
  flowspaceQualifiedName: z.string().optional(),
  policyQualifiedNames: z.array(z.string()).optional(),
  sourceId: z.string(),
  targetId: z.string(),
  connectionType: z.enum(["foundational", "alternative"]),
});

const worldViewGraphModelSchema = z.discriminatedUnion("kind", [
  z.strictObject({
    kind: z.literal("ready"),
    graphBackgroundHex: z.string(),
    graphPattern: z.enum(["none", "grid", "dots"]),
    clusters: z.array(
      z.strictObject({
        backgroundRgba: z.string(),
        badgeBackgroundRgba: z.string(),
        badgeTextHex: z.string(),
        id: z.string(),
        label: z.string(),
        parentId: z.string().optional(),
      }),
    ),
    nodes: z.array(worldViewGraphNodeSchema),
    edges: z.array(worldViewGraphEdgeSchema),
    bounds: z.strictObject({ width: z.number(), height: z.number() }),
  }),
  z.strictObject({
    kind: z.literal("unavailable"),
    reason: z.enum(["missing-realm", "missing-scope", "missing-view"]),
  }),
  z.strictObject({
    kind: z.literal("empty"),
    reason: z.enum(["no-preferences", "no-visible-preferences"]),
    graphBackgroundHex: z.string().optional(),
    graphPattern: z.enum(["none", "grid", "dots"]).optional(),
  }),
]);

const worldViewPresentationModelSchema = z.strictObject({
  graph: worldViewGraphModelSchema,
  revision: z.string().nullable(),
  selection: z.strictObject({
    realmQualifiedName: z.string(),
    scopePreferenceQualifiedName: z.string().optional(),
    viewQualifiedName: z.string(),
  }),
});

const worldViewPresentationSchema = z.strictObject({
  formatVersion: z.literal(1),
  dark: worldViewPresentationModelSchema,
  light: worldViewPresentationModelSchema,
});

const worldViewDumpNodeSchema = z.strictObject({
  preference: z.string(),
  qualifiedName: z.string(),
  status: z.enum(["satisfied", "ready", "blocked"]),
  actionable: z.boolean(),
  leaf: z.boolean(),
  inFocus: z.boolean(),
  inSatisfied: z.boolean(),
  blockers: z.array(z.string()),
  enablers: z.array(z.string()),
  note: z.strictObject({
    preview: z.string().nullable(),
    truncated: z.boolean(),
  }),
  signals: z.array(
    z.object({
      name: z.string(),
      target: z.literal("preference"),
      mode: z.enum(["first", "all"]),
      cases: z.array(
        z.object({
          name: z.string(),
          evidence: z.array(z.looseObject({ kind: z.string() })),
        }),
      ),
    }),
  ),
});

const resolvedWorldViewSchema = z.strictObject({
  formatVersion: z.literal(1),
  bindingId: z.uuid(),
  channelId: z.uuid(),
  declaredScope: worldViewBindingScopeSchema,
  effectiveScope: worldViewBindingScopeSchema,
  bindingRevisionEventId: z.string().regex(/^[0-9a-f]{64}$/),
  sourceRevision: z.string().min(1),
  freshness: z.enum(["pinned", "latest-at-resolution"]),
  authority: z.discriminatedUnion("kind", [
    z.strictObject({
      kind: z.literal("hosted-world-latest"),
      origin: z.url(),
      hostedWorldId: z.string().min(1),
    }),
    z.strictObject({
      kind: z.literal("hosted-world-view-export"),
      origin: z.url(),
    }),
    z.strictObject({
      kind: z.literal("hosted-world-live-view-share"),
      origin: z.url(),
      hostedWorldId: z.string().min(1),
    }),
    z.strictObject({
      kind: z.literal("local-world-mirror-latest"),
      origin: z.url(),
      mirrorId: z.string().min(1),
    }),
  ]),
  realm: worldViewEntitySchema,
  view: worldViewEntitySchema,
  viewDump: z.strictObject({
    counts: z.strictObject({
      nodes: z.number().int().nonnegative(),
      edges: z.number().int().nonnegative(),
      ready: z.number().int().nonnegative(),
      actionableReady: z.number().int().nonnegative(),
      satisfied: z.number().int().nonnegative(),
      blocked: z.number().int().nonnegative(),
    }),
    nodes: z.array(worldViewDumpNodeSchema),
    readyLeaves: z.array(worldViewDumpNodeSchema),
    satisfiedNodes: z.array(worldViewDumpNodeSchema),
    blockedNodes: z.array(worldViewDumpNodeSchema),
    edges: z.array(
      z.strictObject({
        downstream: z.string(),
        upstream: z.string(),
        relation: z.enum(["blocker", "enabler"]),
        connectionType: z.enum(["foundational", "alternative"]),
        flowspace: z.string(),
        flowspaceQualifiedName: z.string(),
      }),
    ),
  }),
  presentation: worldViewPresentationSchema,
  resolvedAt: z.iso.datetime({ offset: true }),
  nextCommand: z.string().min(1),
});

export type ResolvedWorldView = z.infer<typeof resolvedWorldViewSchema>;

type PresentationContractCheck =
  ResolvedWorldView["presentation"] extends WorldViewTileSurfaceProps["presentation"]
    ? true
    : never;
const presentationContractCheck: PresentationContractCheck = true;
void presentationContractCheck;

export function decodeWorldViewAuthorityList(
  value: unknown,
): WorldViewAuthorityListResult {
  return worldViewAuthorityListResultSchema.parse(value);
}

export function decodeSetWorldOriginTrustResult(
  value: unknown,
): SetWorldOriginTrustResult {
  return setWorldOriginTrustResultSchema.parse(value);
}

export function decodeWorldViewMutationDelegations(
  value: unknown,
): WorldViewMutationDelegationListResult {
  return worldViewMutationDelegationListResultSchema.parse(value);
}

export function decodeAuthorizeWorldViewMutationResult(
  value: unknown,
): AuthorizeWorldViewMutationResult {
  return authorizeWorldViewMutationResultSchema.parse(value);
}

export function decodeRevokeWorldViewMutationResult(
  value: unknown,
): RevokeWorldViewMutationResult {
  return revokeWorldViewMutationResultSchema.parse(value);
}

export function decodeConnectLocalWorldAuthorityResult(
  value: unknown,
): ConnectLocalWorldAuthorityResult {
  return connectLocalWorldAuthorityResultSchema.parse(value);
}

export function decodeWorldViewCatalog(value: unknown): WorldViewCatalog {
  return worldViewCatalogSchema.parse(value);
}

export function decodePublishedHostedWorldLiveViewShare(
  value: unknown,
): PublishedHostedWorldLiveViewShare {
  return publishedHostedWorldLiveViewShareSchema.parse(value);
}

export function decodeResolvedWorldView(value: unknown): ResolvedWorldView {
  return resolvedWorldViewSchema.parse(value);
}

export function decodeEffectiveWorldViewBindings(
  value: unknown,
): EffectiveWorldViewBindings {
  return effectiveWorldViewBindingsSchema.parse(value);
}
