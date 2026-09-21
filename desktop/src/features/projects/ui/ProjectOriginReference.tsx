import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  isChannelReferenceOpenable,
  useChannelReference,
} from "@/features/channels/openChannelDirectory";
import { useTranslation } from "@/i18n";

export function ProjectOriginReference({
  agentName,
  channelId,
}: {
  agentName?: string | null;
  channelId?: string | null;
}) {
  const { t } = useTranslation();
  const { goChannel } = useAppNavigation();
  const channel = useChannelReference(channelId);

  if (channelId) {
    const isOpenable = isChannelReferenceOpenable(channel);
    return (
      <span
        className="inline-flex max-w-full min-w-0 items-center gap-1"
        title={
          isOpenable
            ? t("projects.origin-reference.not-verified")
            : // Resolved via a bounded per-id lookup ({@link useChannelReference}),
              // so an unresolved id is a private, deleted, or otherwise
              // inaccessible channel — a non-member open channel still names it.
              `${t("projects.origin-reference.channel-invisible", {
                id: channelId,
              })} ${t("projects.origin-reference.not-verified")}`
        }
      >
        <span
          className="shrink-0 whitespace-nowrap"
          data-project-metadata-phrase
        >
          {t("projects.origin-reference.started-from")}
        </span>
        {isOpenable ? (
          <button
            aria-label={t(
              "projects.origin-reference.open-origin-channel-aria",
              {
                name: channel.name,
              },
            )}
            className="min-w-0 truncate font-medium text-foreground underline-offset-2 hover:underline"
            onClick={() => void goChannel(channel.id)}
            type="button"
          >
            #{channel.name}
          </button>
        ) : (
          <span className="shrink-0 whitespace-nowrap">a private channel</span>
        )}
        <span
          className="shrink-0 whitespace-nowrap"
          data-project-metadata-phrase
        >
          (author-claimed)
        </span>
      </span>
    );
  }

  if (agentName) {
    return (
      <span
        className="inline-flex max-w-full min-w-0 items-center gap-1"
        title={t("projects.origin-reference.private-omitted")}
      >
        <span
          className="shrink-0 whitespace-nowrap"
          data-project-metadata-phrase
        >
          started privately with
        </span>
        <span className="truncate font-medium text-foreground">
          {agentName}
        </span>
      </span>
    );
  }

  return null;
}
