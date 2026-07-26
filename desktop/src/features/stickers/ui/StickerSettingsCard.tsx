import * as React from "react";
import {
  Check,
  ImagePlus,
  PackagePlus,
  Pencil,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import { toast } from "sonner";

import { useMyRelayMembershipQuery } from "@/features/community-members/hooks";
import { SettingsOptionGroup } from "@/features/settings/ui/SettingsOptionGroup";
import { SettingsSectionHeader } from "@/features/settings/ui/SettingsSectionHeader";
import {
  allStickerPacksQueryKey,
  ownStickerPacksQueryKey,
  stickerCatalogQueryKey,
  useAllStickerPacksQuery,
  useInstalledStickerCoordinatesQuery,
  useOwnStickerPacksQuery,
  useSetStickerCatalogApprovalMutation,
  useSetStickerPackInstalledMutation,
  useStickerCatalogQuery,
} from "@/features/stickers/hooks";
import {
  importSignalStickerPack,
  pickAndUploadStickerImage,
} from "@/shared/api/stickersTauri";
import {
  publishStickerPack,
  stickerAssetCacheUrl,
  type StickerAsset,
  type StickerPack,
} from "@/shared/api/stickers";
import { useQueryClient } from "@tanstack/react-query";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";

function PackPreview({
  pack,
  previewAvailable = true,
}: {
  pack: StickerPack;
  previewAvailable?: boolean;
}) {
  return (
    <div className="flex min-w-0 flex-1 items-center gap-3">
      <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-xl bg-muted/50">
        {previewAvailable && pack.stickers[0] ? (
          <img
            alt=""
            className="h-10 w-10 object-contain"
            loading="lazy"
            src={rewriteRelayUrl(stickerAssetCacheUrl(pack, pack.stickers[0]))}
          />
        ) : (
          <ShieldCheck
            aria-label="Preview available after approval"
            className="h-5 w-5 text-muted-foreground"
          />
        )}
      </div>
      <div className="min-w-0">
        <p className="truncate text-sm font-medium">{pack.title}</p>
        <p className="text-xs text-muted-foreground">
          {pack.stickers.length} sticker{pack.stickers.length === 1 ? "" : "s"}
        </p>
        {!previewAvailable ? (
          <p className="text-xs text-muted-foreground">
            Preview available after approval
          </p>
        ) : null}
      </div>
    </div>
  );
}

function PackAuthorForm({ onPublished }: { onPublished: () => void }) {
  const ownPacks = useOwnStickerPacksQuery().data ?? [];
  const [identifier, setIdentifier] = React.useState("");
  const [title, setTitle] = React.useState("");
  const [description, setDescription] = React.useState("");
  const [license, setLicense] = React.useState("");
  const [cover, setCover] = React.useState<StickerPack["cover"]>();
  const [stickers, setStickers] = React.useState<StickerAsset[]>([]);
  const [signalLink, setSignalLink] = React.useState("");
  const [isWorking, setIsWorking] = React.useState(false);

  const loadPack = React.useCallback((pack: StickerPack) => {
    setIdentifier(pack.identifier);
    setTitle(pack.title);
    setDescription(pack.description ?? "");
    setLicense(pack.license ?? "");
    setCover(pack.cover);
    setStickers(pack.stickers);
  }, []);

  const addNativeAsset = React.useCallback(async () => {
    setIsWorking(true);
    try {
      const blob = await pickAndUploadStickerImage();
      if (!blob) return;
      if (
        !["image/png", "image/webp", "image/gif", "image/apng"].includes(
          blob.type,
        )
      ) {
        toast.error("Sonar stickers must be PNG, WebP, APNG, or GIF.");
        return;
      }
      if (!blob.url.startsWith("https://")) {
        toast.error("Sonar sticker assets require an HTTPS relay URL.");
        return;
      }
      const base =
        (blob.filename ?? `sticker-${stickers.length + 1}`)
          .replace(/\.[^.]+$/, "")
          .toLowerCase()
          .replace(/[^a-z0-9_]+/g, "_")
          .replace(/^_+|_+$/g, "") || `sticker_${stickers.length + 1}`;
      const [width, height] = blob.dim?.split("x").map(Number) ?? [];
      setStickers((current) => [
        ...current,
        {
          shortcode: base,
          url: blob.url,
          sha256: blob.sha256,
          mime: blob.type,
          ...(width && height ? { width, height } : {}),
          alt: base.replace(/[_-]+/g, " "),
        },
      ]);
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Could not add sticker.",
      );
    } finally {
      setIsWorking(false);
    }
  }, [stickers.length]);

  const chooseCover = React.useCallback(async () => {
    setIsWorking(true);
    try {
      const blob = await pickAndUploadStickerImage(true);
      if (!blob) return;
      setCover({
        url: blob.url,
        sha256: blob.sha256,
        ...(blob.dim ? { dim: blob.dim } : {}),
      });
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Could not add pack cover.",
      );
    } finally {
      setIsWorking(false);
    }
  }, []);

  const importSignal = React.useCallback(async () => {
    if (!signalLink.trim()) return;
    setIsWorking(true);
    try {
      const imported = await importSignalStickerPack(signalLink);
      setIdentifier(imported.identifier);
      setTitle(imported.title);
      setDescription(
        imported.author
          ? `Imported from Signal · ${imported.author}`
          : "Imported from Signal",
      );
      setCover(
        imported.cover
          ? {
              url: imported.cover.url,
              sha256: imported.cover.sha256,
              ...(imported.cover.width && imported.cover.height
                ? { dim: `${imported.cover.width}x${imported.cover.height}` }
                : {}),
            }
          : undefined,
      );
      setStickers(imported.stickers);
      if (imported.skippedStickerIds.length > 0) {
        toast.warning(
          `Skipped ${imported.skippedStickerIds.length} unavailable sticker(s).`,
        );
      }
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : "Could not import Signal pack.",
      );
    } finally {
      // The secret-bearing link exists only for the duration of this invoke.
      setSignalLink("");
      setIsWorking(false);
    }
  }, [signalLink]);

  const publish = React.useCallback(async () => {
    setIsWorking(true);
    try {
      await publishStickerPack({
        identifier,
        title,
        description,
        license,
        cover,
        stickers,
      });
      toast.success("Sticker pack published.");
      onPublished();
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Could not publish pack.",
      );
    } finally {
      setIsWorking(false);
    }
  }, [cover, description, identifier, license, onPublished, stickers, title]);

  return (
    <SettingsOptionGroup className="space-y-4 p-4">
      <div>
        <h3 className="text-sm font-medium">Create or edit a pack</h3>
        <p className="mt-1 text-xs text-muted-foreground">
          Choose one of your packs to edit, upload images, or import a Signal
          pack.
        </p>
      </div>
      {ownPacks.length > 0 ? (
        <div className="flex flex-wrap gap-2">
          {ownPacks.map((pack) => (
            <Button
              key={pack.coordinate}
              onClick={() => loadPack(pack)}
              size="sm"
              type="button"
              variant="outline"
            >
              <Pencil className="mr-1 h-3.5 w-3.5" />
              {pack.title}
            </Button>
          ))}
        </div>
      ) : null}
      <div className="grid gap-3 sm:grid-cols-2">
        <Input
          aria-label="Pack ID"
          maxLength={80}
          onChange={(event) => setIdentifier(event.target.value)}
          placeholder="pack-id"
          value={identifier}
        />
        <Input
          aria-label="Pack title"
          maxLength={80}
          onChange={(event) => setTitle(event.target.value)}
          placeholder="Pack title"
          value={title}
        />
      </div>
      <Textarea
        aria-label="Pack description"
        maxLength={500}
        onChange={(event) => setDescription(event.target.value)}
        placeholder="Description (optional)"
        value={description}
      />
      <Input
        aria-label="Pack license"
        maxLength={160}
        onChange={(event) => setLicense(event.target.value)}
        placeholder="License (optional)"
        value={license}
      />
      <div className="flex flex-wrap items-center gap-2">
        <Button
          disabled={isWorking}
          onClick={() => void chooseCover()}
          type="button"
          variant="outline"
        >
          <ImagePlus className="mr-2 h-4 w-4" />
          {cover ? "Replace WebP cover" : "Add WebP cover"}
        </Button>
        {cover ? (
          <>
            <span className="text-xs text-muted-foreground">
              {cover.dim ?? "WebP cover selected"}
            </span>
            <Button
              aria-label="Remove pack cover"
              onClick={() => setCover(undefined)}
              size="icon"
              type="button"
              variant="ghost"
            >
              <Trash2 className="h-4 w-4" />
            </Button>
          </>
        ) : null}
      </div>
      <div className="flex gap-2">
        <Input
          aria-label="Signal sticker link"
          autoComplete="off"
          onChange={(event) => setSignalLink(event.target.value)}
          placeholder="https://signal.art/addstickers/#pack_id=…"
          type="password"
          value={signalLink}
        />
        <Button
          disabled={isWorking || !signalLink.trim()}
          onClick={() => void importSignal()}
          type="button"
          variant="secondary"
        >
          Import Signal
        </Button>
      </div>
      <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
        {stickers.map((sticker, index) => (
          <div
            className="group relative rounded-xl bg-background p-2"
            key={sticker.sha256}
          >
            <div className="flex aspect-square w-full items-center justify-center rounded-lg bg-muted/40 text-xs text-muted-foreground">
              {sticker.emoji ?? `:${sticker.shortcode}:`}
            </div>
            <Input
              aria-label={`Shortcode ${index + 1}`}
              className="mt-1 h-7 px-1 text-xs"
              onChange={(event) =>
                setStickers((current) =>
                  current.map((value, itemIndex) =>
                    itemIndex === index
                      ? { ...value, shortcode: event.target.value }
                      : value,
                  ),
                )
              }
              value={sticker.shortcode}
            />
            <Input
              aria-label={`Alt text ${index + 1}`}
              className="mt-1 h-7 px-1 text-xs"
              maxLength={160}
              onChange={(event) =>
                setStickers((current) =>
                  current.map((value, itemIndex) =>
                    itemIndex === index
                      ? { ...value, alt: event.target.value }
                      : value,
                  ),
                )
              }
              placeholder="Alt text"
              value={sticker.alt ?? ""}
            />
            <Input
              aria-label={`Representative emoji ${index + 1}`}
              className="mt-1 h-7 px-1 text-xs"
              maxLength={8}
              onChange={(event) =>
                setStickers((current) =>
                  current.map((value, itemIndex) =>
                    itemIndex === index
                      ? { ...value, emoji: event.target.value }
                      : value,
                  ),
                )
              }
              placeholder="Emoji (optional)"
              value={sticker.emoji ?? ""}
            />
            <button
              aria-label={`Remove ${sticker.shortcode}`}
              className="absolute right-1 top-1 rounded-full bg-background/90 p-1 opacity-0 shadow group-hover:opacity-100"
              onClick={() =>
                setStickers((current) =>
                  current.filter((_, itemIndex) => itemIndex !== index),
                )
              }
              type="button"
            >
              <Trash2 className="h-3.5 w-3.5" />
            </button>
          </div>
        ))}
      </div>
      <div className="flex flex-wrap gap-2">
        <Button
          disabled={isWorking || stickers.length >= 200}
          onClick={() => void addNativeAsset()}
          type="button"
          variant="outline"
        >
          <ImagePlus className="mr-2 h-4 w-4" />
          Add image
        </Button>
        <Button
          disabled={isWorking || !identifier || !title || stickers.length === 0}
          onClick={() => void publish()}
          type="button"
        >
          <PackagePlus className="mr-2 h-4 w-4" />
          Publish pack
        </Button>
      </div>
    </SettingsOptionGroup>
  );
}

export function StickerSettingsCard() {
  const queryClient = useQueryClient();
  const catalogQuery = useStickerCatalogQuery();
  const installedQuery = useInstalledStickerCoordinatesQuery();
  const membership = useMyRelayMembershipQuery().data;
  const canCurate =
    membership?.role === "owner" || membership?.role === "admin";
  const allPacksQuery = useAllStickerPacksQuery(canCurate);
  const setInstalled = useSetStickerPackInstalledMutation();
  const setApproval = useSetStickerCatalogApprovalMutation();
  const installed = React.useMemo(
    () => new Set(installedQuery.data ?? []),
    [installedQuery.data],
  );
  const approved = React.useMemo(
    () =>
      new Map(
        (catalogQuery.data ?? []).map((pack) => [
          pack.coordinate,
          pack.eventId,
        ]),
      ),
    [catalogQuery.data],
  );
  const pending = (allPacksQuery.data ?? []).filter(
    (pack) => approved.get(pack.coordinate) !== pack.eventId,
  );

  const invalidatePublished = React.useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: ownStickerPacksQueryKey });
    void queryClient.invalidateQueries({ queryKey: allStickerPacksQueryKey });
    void queryClient.invalidateQueries({ queryKey: stickerCatalogQueryKey });
  }, [queryClient]);

  return (
    <section className="min-w-0" data-testid="settings-stickers">
      <SettingsSectionHeader
        title="Stickers"
        description="Install curated Sonar sticker packs, or publish one of your own."
      />
      <div className="space-y-6">
        <SettingsOptionGroup>
          {(catalogQuery.data ?? []).map((pack) => {
            const isInstalled = installed.has(pack.coordinate);
            return (
              <div
                className="flex items-center gap-3 border-b border-border/40 px-4 py-3 last:border-0"
                key={pack.coordinate}
              >
                <PackPreview pack={pack} />
                <Button
                  disabled={setInstalled.isPending}
                  onClick={() =>
                    void setInstalled
                      .mutateAsync({
                        coordinate: pack.coordinate,
                        installed: !isInstalled,
                      })
                      .catch((error) => toast.error(error.message))
                  }
                  size="sm"
                  type="button"
                  variant={isInstalled ? "secondary" : "default"}
                >
                  {isInstalled ? (
                    <>
                      <Check className="mr-1 h-4 w-4" />
                      Installed
                    </>
                  ) : (
                    "Install"
                  )}
                </Button>
                {canCurate ? (
                  <Button
                    aria-label={`Remove ${pack.title} from catalog`}
                    disabled={setApproval.isPending}
                    onClick={() =>
                      void setApproval
                        .mutateAsync({
                          coordinate: pack.coordinate,
                          eventId: pack.eventId,
                          approved: false,
                        })
                        .catch((error) => toast.error(error.message))
                    }
                    size="icon"
                    type="button"
                    variant="ghost"
                  >
                    <Trash2 className="h-4 w-4" />
                  </Button>
                ) : null}
              </div>
            );
          })}
          {!catalogQuery.isLoading && (catalogQuery.data?.length ?? 0) === 0 ? (
            <p className="px-4 py-6 text-center text-sm text-muted-foreground">
              No sticker packs have been approved yet.
            </p>
          ) : null}
        </SettingsOptionGroup>

        {canCurate && pending.length > 0 ? (
          <SettingsOptionGroup className="p-4">
            <h3 className="mb-3 text-sm font-medium">
              Awaiting catalog approval
            </h3>
            <div className="space-y-2">
              {pending.map((pack) => (
                <div
                  className="flex items-center gap-3"
                  key={`${pack.coordinate}:${pack.eventId}`}
                >
                  <PackPreview pack={pack} previewAvailable={false} />
                  <Button
                    disabled={setApproval.isPending}
                    onClick={() =>
                      void setApproval
                        .mutateAsync({
                          coordinate: pack.coordinate,
                          eventId: pack.eventId,
                          approved: true,
                        })
                        .catch((error) => toast.error(error.message))
                    }
                    size="sm"
                    type="button"
                    variant="outline"
                  >
                    <ShieldCheck className="mr-1 h-4 w-4" />
                    Approve
                  </Button>
                </div>
              ))}
            </div>
          </SettingsOptionGroup>
        ) : null}

        <PackAuthorForm onPublished={invalidatePublished} />
      </div>
    </section>
  );
}
