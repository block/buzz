import * as React from "react";
import { PackageOpen, Settings, Sticker as StickerIcon } from "lucide-react";

import {
  useInstalledStickerPacks,
  useSetStickerPackInstalledMutation,
  useStickerCatalogQuery,
} from "@/features/stickers/hooks";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { toast } from "sonner";
import {
  stickerAssetCacheUrl,
  type StickerAsset,
  type StickerPack,
} from "@/shared/api/stickers";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { Button } from "@/shared/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

export type StickerSelection = { pack: StickerPack; sticker: StickerAsset };

export const ComposerStickerPicker = React.memo(function ComposerStickerPicker({
  disabled,
  onSelect,
}: {
  disabled?: boolean;
  onSelect: (selection: StickerSelection) => void;
}) {
  const packs = useInstalledStickerPacks();
  const catalogPacks = useStickerCatalogQuery().data ?? [];
  // Superseded placeholders are not installable.
  const installableCatalogPacks = catalogPacks.filter(
    (pack) => !pack.superseded,
  );
  const setInstalled = useSetStickerPackInstalledMutation();
  const [installingPack, setInstallingPack] = React.useState<string | null>(
    null,
  );
  const { goSettings } = useAppNavigation();
  const installPack = React.useCallback(
    async (coordinate: string) => {
      setInstallingPack(coordinate);
      try {
        await setInstalled.mutateAsync({ coordinate, installed: true });
      } finally {
        setInstallingPack(null);
      }
    },
    [setInstalled],
  );
  const [open, setOpen] = React.useState(false);
  const [selectedCoordinate, setSelectedCoordinate] = React.useState("");
  const selectedPack =
    packs.find((pack) => pack.coordinate === selectedCoordinate) ?? packs[0];
  return (
    <Popover onOpenChange={setOpen} open={open}>
      <Tooltip disableHoverableContent>
        <TooltipTrigger asChild>
          <PopoverTrigger asChild>
            <Button
              aria-label="Choose sticker"
              data-testid="composer-sticker-button"
              disabled={disabled}
              size="icon"
              type="button"
              variant="ghost"
            >
              <StickerIcon />
            </Button>
          </PopoverTrigger>
        </TooltipTrigger>
        <TooltipContent>Choose sticker</TooltipContent>
      </Tooltip>
      <PopoverContent
        align="start"
        className="max-h-80 w-80 overflow-y-auto rounded-2xl p-3"
        side="top"
        sideOffset={10}
      >
        {packs.length === 0 ? (
          <div className="space-y-3">
            <div className="flex flex-col items-center gap-1 px-2 pt-1 text-center">
              <PackageOpen className="h-6 w-6 text-muted-foreground" />
              <p className="text-sm font-medium">No stickers installed</p>
              <p className="text-xs text-muted-foreground">
                Install a curated pack, or create your own.
              </p>
            </div>
            {installableCatalogPacks.length > 0 ? (
              <div className="max-h-48 space-y-1 overflow-y-auto">
                {installableCatalogPacks.map((pack) => {
                  const installing = installingPack === pack.coordinate;
                  return (
                    <div
                      className="flex items-center justify-between gap-2 rounded-md px-2 py-1.5"
                      key={pack.coordinate}
                    >
                      <span className="min-w-0 truncate text-xs">
                        {pack.title}
                      </span>
                      <Button
                        disabled={installing}
                        onClick={() =>
                          void installPack(pack.coordinate).catch((error) =>
                            toast.error(
                              error instanceof Error
                                ? error.message
                                : "Could not install pack.",
                            ),
                          )
                        }
                        size="sm"
                        type="button"
                        variant="default"
                      >
                        {installing ? "Installing…" : "Install"}
                      </Button>
                    </div>
                  );
                })}
              </div>
            ) : null}
            <Button
              className="w-full"
              onClick={() => {
                setOpen(false);
                goSettings("stickers");
              }}
              size="sm"
              type="button"
              variant="outline"
            >
              <Settings className="mr-2 h-4 w-4" />
              Open sticker settings
            </Button>
          </div>
        ) : (
          <div className="space-y-3">
            <label className="block">
              <span className="sr-only">Sticker pack</span>
              <select
                className="h-8 w-full rounded-md border border-input bg-background px-2 text-sm"
                data-testid="composer-sticker-pack-select"
                onChange={(event) => setSelectedCoordinate(event.target.value)}
                value={selectedPack?.coordinate ?? ""}
              >
                {packs.map((pack) => (
                  <option key={pack.coordinate} value={pack.coordinate}>
                    {pack.title}
                  </option>
                ))}
              </select>
            </label>
            {selectedPack ? (
              <section key={selectedPack.coordinate}>
                <div className="grid grid-cols-5 gap-1">
                  {selectedPack.stickers.map((sticker) => (
                    <button
                      aria-label={`Send ${sticker.shortcode}`}
                      className="rounded-lg p-1 transition-colors hover:bg-muted focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
                      key={sticker.sha256}
                      onClick={() => {
                        onSelect({ pack: selectedPack, sticker });
                        setOpen(false);
                      }}
                      title={`:${sticker.shortcode}:`}
                      type="button"
                    >
                      <img
                        alt={sticker.alt ?? ""}
                        className="aspect-square w-full object-contain"
                        loading="lazy"
                        src={rewriteRelayUrl(
                          stickerAssetCacheUrl(selectedPack, sticker),
                        )}
                      />
                    </button>
                  ))}
                </div>
              </section>
            ) : null}
          </div>
        )}
      </PopoverContent>
    </Popover>
  );
});
