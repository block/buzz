// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT, useUiLanguage } from "@/shared/i18n/uiTranslation";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  isChannelReferenceOpenable,
  useChannelReference,
} from "@/features/channels/openChannelDirectory";

export function ProjectOriginReference({
  agentName,
  channelId,
}: {
  agentName?: string | null;
  channelId?: string | null;
}) {
  useUiLanguage();

  const { goChannel } = useAppNavigation();
  const channel = useChannelReference(channelId);

  if (channelId) {
    const isOpenable = isChannelReferenceOpenable(channel);
    return (
      <span
        className="inline-flex max-w-full min-w-0 items-center gap-1"
        title={
          isOpenable
            ? uiT(
                "Origin is claimed by the event author and is not relay-verified.",
              )
            : // Resolved via a bounded per-id lookup ({@link useChannelReference}),
              // so an unresolved id is a private, deleted, or otherwise
              // inaccessible channel — a non-member open channel still names it.
              uiT(
                "Origin channel {0} is not visible to you. Origin is claimed by the event author and is not relay-verified.",
                { p0: channelId },
              )
        }
      >
        <span
          className="shrink-0 whitespace-nowrap"
          data-project-metadata-phrase
        >
          {uiT("started from")}
        </span>
        {isOpenable ? (
          <button
            aria-label={uiT("Open author-claimed origin channel #{0}", {
              p0: channel.name,
            })}
            className="min-w-0 truncate font-medium text-foreground underline-offset-2 hover:underline"
            onClick={() => void goChannel(channel.id)}
            type="button"
          >
            #{channel.name}
          </button>
        ) : (
          <span className="shrink-0 whitespace-nowrap">
            {uiT("a private channel")}
          </span>
        )}
        <span
          className="shrink-0 whitespace-nowrap"
          data-project-metadata-phrase
        >
          {uiT("(author-claimed)")}
        </span>
      </span>
    );
  }

  if (agentName) {
    return (
      <span
        className="inline-flex max-w-full min-w-0 items-center gap-1"
        title={uiT(
          "The private conversation identifier is intentionally omitted.",
        )}
      >
        <span
          className="shrink-0 whitespace-nowrap"
          data-project-metadata-phrase
        >
          {uiT("started privately with")}
        </span>
        <span className="truncate font-medium text-foreground">
          {agentName}
        </span>
      </span>
    );
  }

  return null;
}
