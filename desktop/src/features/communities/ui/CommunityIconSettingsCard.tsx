import { useMutation, useQueryClient } from "@tanstack/react-query";
import * as React from "react";
import { toast } from "sonner";

import { AgentCreationPreview } from "@/features/agents/ui/AgentCreationPreview";
import { downscaleIconToDataUrl } from "@/features/communities/lib/downscaleIcon";
import {
  communityIconQueryKey,
  useActiveCommunityIcon,
} from "@/features/communities/useCommunityIcons";
import { useCommunities } from "@/features/communities/useCommunities";
import { setCommunityIcon } from "@/shared/api/communityProfile";

/**
 * Community icon editor using the same image/emoji picker as agent creation.
 * Images stay as small inline data URLs so inactive-community rails can render
 * them without crossing another relay's authenticated media boundary.
 */
export function CommunityIconSettingsCard({
  compact = false,
}: {
  compact?: boolean;
}) {
  const { activeCommunity } = useCommunities();
  const relayUrl = activeCommunity?.relayUrl;
  const iconQuery = useActiveCommunityIcon(relayUrl);
  const queryClient = useQueryClient();
  const persistedIcon = iconQuery.data ?? "";
  const [draftIcon, setDraftIcon] = React.useState(persistedIcon);
  const queuedIconRef = React.useRef<string | undefined>(undefined);
  const isSavingRef = React.useRef(false);
  const hasLocalEditRef = React.useRef(false);

  React.useEffect(() => {
    if (hasLocalEditRef.current) return;
    setDraftIcon(persistedIcon);
  }, [persistedIcon]);

  const mutation = useMutation({
    mutationFn: (icon: string) => setCommunityIcon(icon),
    onSuccess: async (_data, icon) => {
      if (!relayUrl) return;
      queryClient.setQueryData(communityIconQueryKey(relayUrl), icon || null);
      await queryClient.invalidateQueries({
        queryKey: communityIconQueryKey(relayUrl),
      });
    },
  });

  const mutateIcon = mutation.mutateAsync;

  const persistIcon = React.useCallback(
    (icon: string) => {
      hasLocalEditRef.current = true;
      queuedIconRef.current = icon;
      setDraftIcon(icon);
      if (isSavingRef.current) return;

      isSavingRef.current = true;
      const drainQueue = async () => {
        while (queuedIconRef.current !== undefined) {
          const nextIcon = queuedIconRef.current;
          queuedIconRef.current = undefined;
          try {
            await mutateIcon(nextIcon);
          } catch (error) {
            toast.error(
              error instanceof Error
                ? error.message
                : "Couldn’t update the community icon.",
            );
          }
        }
        isSavingRef.current = false;
      };
      void drainQueue();
    },
    [mutateIcon],
  );

  function previewIcon(icon: string) {
    hasLocalEditRef.current = true;
    setDraftIcon(icon);
  }

  function clearIconPreview() {
    hasLocalEditRef.current = true;
    setDraftIcon("");
  }

  return (
    <div
      className={
        compact
          ? "flex shrink-0 items-center"
          : "flex min-w-0 flex-col items-center"
      }
      data-testid="community-icon-settings"
    >
      <AgentCreationPreview
        assetLabel="community icon"
        avatarUrl={draftIcon || null}
        label={activeCommunity?.name ?? "Community"}
        onClearAvatar={clearIconPreview}
        onCommitAvatar={persistIcon}
        onSelectAvatar={previewIcon}
        processImage={downscaleIconToDataUrl}
        shape="rounded-square"
        testIdPrefix="community-icon"
        variant={compact ? "compact" : "default"}
      />
    </div>
  );
}
