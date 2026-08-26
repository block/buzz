import { Globe, Pencil, Plus, Trash2 } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import {
  useChannelWebsitesQuery,
  useSetChannelWebsitesMutation,
} from "@/features/channels/hooks";
import { fetchChannelWebsitePageTitle } from "@/features/channels/lib/channelWebsiteTitle";
import {
  channelWebsiteTabLabel,
  type ChannelWebsite,
  normalizeChannelWebsiteUrl,
  validateChannelWebsiteDraft,
} from "@/features/channels/lib/channelWebsites";
import { ChannelWebsiteFavicon } from "@/features/channels/ui/ChannelWebsiteFavicon";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import {
  CHANNEL_FORM_FIELD_CONTROL_CLASS,
  CHANNEL_FORM_FIELD_SHELL_CLASS,
} from "./channelFormStyles";

type ChannelWebsitesSettingsProps = {
  channelId: string;
  enabled?: boolean;
};

function newWebsiteId(): string {
  return crypto.randomUUID();
}

export function ChannelWebsitesSettings({
  channelId,
  enabled = true,
}: ChannelWebsitesSettingsProps) {
  const websitesQuery = useChannelWebsitesQuery(channelId, enabled);
  const saveMutation = useSetChannelWebsitesMutation(channelId);
  const websites = websitesQuery.data ?? [];

  const [draftTitle, setDraftTitle] = React.useState("");
  const [draftUrl, setDraftUrl] = React.useState("");
  const [editingId, setEditingId] = React.useState<string | null>(null);
  const [titleLookup, setTitleLookup] = React.useState<"idle" | "loading">(
    "idle",
  );
  const urlInputRef = React.useRef<HTMLInputElement>(null);
  const attemptedTitleIds = React.useRef(new Set<string>());
  const titleTouchedRef = React.useRef(false);

  const persist = React.useCallback(
    async (next: ChannelWebsite[], quiet = false) => {
      try {
        await saveMutation.mutateAsync(next);
        if (!quiet) toast.success("Channel websites updated");
      } catch (error) {
        toast.error(
          error instanceof Error ? error.message : "Failed to save websites",
        );
      }
    },
    [saveMutation],
  );

  const resetDraft = React.useCallback(() => {
    titleTouchedRef.current = false;
    setDraftTitle("");
    setDraftUrl("");
    setEditingId(null);
    setTitleLookup("idle");
  }, []);

  const handleAddOrUpdate = React.useCallback(async () => {
    const validated = validateChannelWebsiteDraft({
      title: draftTitle,
      url: draftUrl,
    });
    if (!validated) {
      toast.error(
        draftUrl.trim() || draftTitle.trim()
          ? "Enter a valid https URL"
          : "Type an https URL, then click Add website",
      );
      return;
    }
    const id = editingId ?? newWebsiteId();
    let title = validated.title;
    if (!title) {
      title = (await fetchChannelWebsitePageTitle(validated.url)) ?? "";
    }
    const entry: ChannelWebsite = {
      id,
      title,
      url: validated.url,
    };
    const without = editingId
      ? websites.filter((site) => site.id !== editingId)
      : websites;
    await persist([...without, entry]);
    resetDraft();
  }, [draftTitle, draftUrl, editingId, persist, resetDraft, websites]);

  const handleEdit = React.useCallback((site: ChannelWebsite) => {
    titleTouchedRef.current = Boolean(site.title.trim());
    setEditingId(site.id);
    setDraftTitle(site.title);
    setDraftUrl(site.url);
    window.setTimeout(() => {
      urlInputRef.current?.focus();
      urlInputRef.current?.select();
    }, 0);
  }, []);

  React.useEffect(() => {
    if (!enabled || titleTouchedRef.current) return;
    const url = normalizeChannelWebsiteUrl(draftUrl);
    if (!url) {
      setTitleLookup("idle");
      return;
    }
    let cancelled = false;
    setTitleLookup("loading");
    const timer = window.setTimeout(() => {
      void fetchChannelWebsitePageTitle(url).then((title) => {
        if (cancelled) return;
        setTitleLookup("idle");
        if (title && !titleTouchedRef.current) {
          setDraftTitle(title);
        }
      });
    }, 450);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [draftUrl, enabled]);

  React.useEffect(() => {
    if (!enabled) return;
    const missing = websites.filter(
      (site) => !site.title.trim() && !attemptedTitleIds.current.has(site.id),
    );
    if (missing.length === 0) return;
    for (const site of missing) {
      attemptedTitleIds.current.add(site.id);
    }
    let cancelled = false;
    void (async () => {
      const titles = new Map<string, string>();
      await Promise.all(
        missing.map(async (site) => {
          const title = await fetchChannelWebsitePageTitle(site.url);
          if (title) titles.set(site.id, title);
        }),
      );
      if (cancelled || titles.size === 0) return;
      const next = websites.map((site) => {
        const title = titles.get(site.id);
        return title ? { ...site, title } : site;
      });
      await persist(next, true);
    })();
    return () => {
      cancelled = true;
    };
  }, [enabled, persist, websites]);

  const handleRemove = React.useCallback(
    async (siteId: string) => {
      await persist(websites.filter((site) => site.id !== siteId));
      if (editingId === siteId) resetDraft();
    },
    [editingId, persist, resetDraft, websites],
  );

  return (
    <div className="space-y-4" data-testid="channel-websites-settings">
      <div className="space-y-1">
        <div className="flex items-center gap-2 text-sm font-medium">
          <Globe className="h-4 w-4 text-muted-foreground" />
          Websites
        </div>
        <p className="text-sm text-muted-foreground">
          Add URLs to show as tabs in the channel header. Anyone who can see
          this channel can manage the list.
        </p>
      </div>

      {websites.length > 0 ? (
        <ul className="space-y-2">
          {websites.map((site) => (
            <li
              className={cn(
                "flex items-center justify-between gap-2 rounded-md border px-3 py-2",
                editingId === site.id && "border-foreground/40 bg-muted/40",
              )}
              data-testid={`channel-website-row-${site.id}`}
              key={site.id}
            >
              <div className="flex min-w-0 items-center gap-2">
                <ChannelWebsiteFavicon
                  label={channelWebsiteTabLabel(site)}
                  url={site.url}
                />
                <div className="min-w-0">
                  <p className="truncate text-sm font-medium">
                    {channelWebsiteTabLabel(site)}
                  </p>
                  <p className="truncate text-xs text-muted-foreground">
                    {site.url}
                  </p>
                </div>
              </div>
              <div className="flex shrink-0 items-center gap-1">
                <Button
                  aria-label={`Edit ${channelWebsiteTabLabel(site)}`}
                  data-testid={`channel-website-edit-${site.id}`}
                  onClick={(event) => {
                    event.preventDefault();
                    event.stopPropagation();
                    handleEdit(site);
                  }}
                  size="icon"
                  type="button"
                  variant="ghost"
                >
                  <Pencil className="h-4 w-4" />
                </Button>
                <Button
                  aria-label={`Remove ${channelWebsiteTabLabel(site)}`}
                  onClick={() => void handleRemove(site.id)}
                  size="icon"
                  type="button"
                  variant="ghost"
                >
                  <Trash2 className="h-4 w-4" />
                </Button>
              </div>
            </li>
          ))}
        </ul>
      ) : (
        <p className="text-sm text-muted-foreground">No websites yet.</p>
      )}

      <div className="space-y-2">
        <div className="space-y-1">
          <label
            className="text-xs font-medium text-muted-foreground"
            htmlFor="channel-website-url-input"
          >
            Website URL
          </label>
          <div className={CHANNEL_FORM_FIELD_SHELL_CLASS}>
            <Input
              className={cn(
                CHANNEL_FORM_FIELD_CONTROL_CLASS,
                draftUrl && "text-foreground",
              )}
              data-testid="channel-website-url-input"
              id="channel-website-url-input"
              onChange={(event) => setDraftUrl(event.target.value)}
              placeholder="https://example.com"
              ref={urlInputRef}
              value={draftUrl}
            />
          </div>
        </div>
        <div className="space-y-1">
          <label
            className="text-xs font-medium text-muted-foreground"
            htmlFor="channel-website-title-input"
          >
            Tab label (filled from the page title)
          </label>
          <div className={CHANNEL_FORM_FIELD_SHELL_CLASS}>
            <Input
              className={cn(
                CHANNEL_FORM_FIELD_CONTROL_CLASS,
                draftTitle && "text-foreground",
              )}
              data-testid="channel-website-title-input"
              id="channel-website-title-input"
              onChange={(event) => {
                titleTouchedRef.current = true;
                setDraftTitle(event.target.value);
              }}
              placeholder={
                titleLookup === "loading" ? "Looking up page title…" : "Docs"
              }
              value={draftTitle}
            />
          </div>
        </div>
        <div className="flex items-center gap-2">
          <Button
            data-testid="channel-website-save"
            disabled={saveMutation.isPending || websitesQuery.isLoading}
            onClick={() => void handleAddOrUpdate()}
            size="sm"
            type="button"
          >
            <Plus className="mr-1.5 h-4 w-4" />
            {editingId ? "Update website" : "Add website"}
          </Button>
          {editingId ? (
            <Button
              onClick={resetDraft}
              size="sm"
              type="button"
              variant="ghost"
            >
              Cancel edit
            </Button>
          ) : null}
        </div>
      </div>
    </div>
  );
}
