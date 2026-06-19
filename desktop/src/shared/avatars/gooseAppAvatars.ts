import catalog from "@/shared/assets/goose-avatars/catalog.json";
import {
  GOOSE_APP_AVATAR_REF_PREFIX,
  parseGooseAppAvatarId,
  toGooseAppAvatarRef,
} from "./gooseAppAvatarRefs";

type GooseAvatarVariant = {
  path: string;
  mimeType: string;
  byteSize: number;
  sha256: string;
};

type GooseAvatarCatalogAsset = {
  id: string;
  label: string;
  collectionId: string;
  variants: {
    hevc: GooseAvatarVariant;
    webm: GooseAvatarVariant;
  };
};

type GooseAvatarCatalogCollection = {
  id: string;
  label: string;
  coverAvatarId: string;
  avatarIds: string[];
};

type GooseAvatarCatalog = {
  schemaVersion: 1;
  catalogVersion: string;
  collections: GooseAvatarCatalogCollection[];
  assets: GooseAvatarCatalogAsset[];
};

export type GooseAppAvatarAsset = {
  id: string;
  ref: string;
  label: string;
  collectionId: string;
  posterUrl: string | null;
  webmUrl: string | null;
  hevcUrl: string | null;
};

export type GooseAppAvatarCollection = {
  id: string;
  label: string;
  avatars: GooseAppAvatarAsset[];
};

const typedCatalog = catalog as GooseAvatarCatalog;

const posterModules = import.meta.glob(
  "/src/shared/assets/goose-avatars/posters/**/*.png",
  {
    eager: true,
    import: "default",
    query: "?url",
  },
) as Record<string, string>;

const webmModules = import.meta.glob(
  "/src/shared/assets/goose-avatars/webm/**/*.webm",
  {
    eager: true,
    import: "default",
    query: "?url",
  },
) as Record<string, string>;

const hevcModules = import.meta.glob(
  "/src/shared/assets/goose-avatars/hevc/**/*.mp4",
  {
    eager: true,
    import: "default",
    query: "?url",
  },
) as Record<string, string>;

function moduleUrlFor(
  modules: Record<string, string>,
  collectionId: string,
  id: string,
  extension: string,
): string | null {
  const suffix = `/${collectionId}/${id}.${extension}`;
  return (
    Object.entries(modules).find(([path]) => path.endsWith(suffix))?.[1] ?? null
  );
}

const assetsById = new Map<string, GooseAppAvatarAsset>(
  typedCatalog.assets.map((asset) => [
    asset.id,
    {
      id: asset.id,
      ref: `${GOOSE_APP_AVATAR_REF_PREFIX}${asset.id}`,
      label: asset.label,
      collectionId: asset.collectionId,
      posterUrl: moduleUrlFor(
        posterModules,
        asset.collectionId,
        asset.id,
        "png",
      ),
      webmUrl: moduleUrlFor(webmModules, asset.collectionId, asset.id, "webm"),
      hevcUrl: moduleUrlFor(hevcModules, asset.collectionId, asset.id, "mp4"),
    },
  ]),
);

export const GOOSE_APP_AVATAR_COLLECTIONS: GooseAppAvatarCollection[] =
  typedCatalog.collections.map((collection) => ({
    id: collection.id,
    label: collection.label,
    avatars: collection.avatarIds
      .map((id) => assetsById.get(id))
      .filter((asset): asset is GooseAppAvatarAsset => asset !== undefined),
  }));

const allGooseAppAvatars = GOOSE_APP_AVATAR_COLLECTIONS.flatMap(
  (collection) => collection.avatars,
);

export function getRandomGooseAppAvatarRef(
  random: () => number = Math.random,
): string | null {
  if (allGooseAppAvatars.length === 0) {
    return null;
  }
  const index = Math.floor(random() * allGooseAppAvatars.length);
  return allGooseAppAvatars[index]?.ref ?? null;
}

export function resolveGooseAppAvatar(
  value: string | null | undefined,
): GooseAppAvatarAsset | null {
  const id = parseGooseAppAvatarId(value);
  return id ? (assetsById.get(id) ?? null) : null;
}

export function isResolvedGooseAppAvatar(
  value: string | null | undefined,
): boolean {
  return resolveGooseAppAvatar(value) !== null;
}

export { toGooseAppAvatarRef };
