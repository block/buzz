part of '../search_page.dart';

class _MessageTile extends ConsumerWidget {
  final SearchHit hit;
  final UserProfile? authorProfile;
  final Map<String, UserProfile> userCache;
  final Channel? channel;
  final String? currentPubkey;
  final VoidCallback onResultSelected;

  const _MessageTile({
    required this.hit,
    required this.authorProfile,
    required this.userCache,
    required this.channel,
    required this.currentPubkey,
    required this.onResultSelected,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final mentionedPubkeys = mentionedPubkeysFromTags(hit.tags);
    // A hit is labelled within its channel, as it would be there.
    final hitChannelId = channel?.id ?? hit.channelId;
    final labelPubkeys = {hit.pubkey.toLowerCase(), ...mentionedPubkeys};
    final Map<String, String> labels;
    if (hitChannelId == null) {
      final names = watchIdentityNames(ref, labelPubkeys);
      labels = {
        for (final pubkey in labelPubkeys) pubkey: names.labelFor(pubkey),
      };
    } else {
      labels = watchChannelIdentityLabels(ref, hitChannelId, labelPubkeys);
    }
    final authorName =
        labels[hit.pubkey.toLowerCase()] ??
        authorProfile?.label ??
        shortPubkey(hit.pubkey);
    final timeAgo = relativeTime(hit.createdAt);
    final channelName = hit.channelName?.trim().replaceFirst(RegExp(r'^#'), '');
    final hasChannelName = channelName != null && channelName.isNotEmpty;
    final isDm = channel?.isDm ?? false;
    final profileMentionNames = {
      for (final pubkey in mentionedPubkeysFromTags(hit.tags))
        if (userCache[pubkey]?.displayName?.trim().isNotEmpty == true)
          pubkey: userCache[pubkey]!.displayName!.trim(),
    };
    final mentionPubkeys = mentionedPubkeysFromTags(hit.tags);
    final knownAgentPubkeys = channel == null
        ? ref.watch(knownAgentPubkeysProvider)
        : ref.watch(agentMentionPubkeysProvider(channel!.id));
    final agentMentionPubkeys = agentPubkeysWithProfileOwners(
      knownAgentPubkeys: knownAgentPubkeys,
      profileOwnedAgentPubkeys: [
        for (final profile in userCache.values)
          if (profile.ownerPubkey != null) profile.pubkey,
      ],
    );
    final mentionNames = mentionNamesWithDirectoryLabels(
      mentionPubkeys: mentionPubkeys,
      profileMentionNames: profileMentionNames,
      directoryDisplayNames: ref.watch(agentDirectoryDisplayNamesProvider),
      agentMentionPubkeys: agentMentionPubkeys,
    );

    return ListTile(
      key: ValueKey('search-message-row-${hit.eventId}'),
      contentPadding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
      titleAlignment: ListTileTitleAlignment.top,
      horizontalTitleGap: messageAvatarContentGap,
      leading: SmallAvatar(
        key: ValueKey('search-message-avatar-${hit.eventId}'),
        pubkey: hit.pubkey,
        userCache: userCache,
        size: compactMessageAvatarSize,
      ),
      title: MessageAuthorMeta(
        displayName: authorName,
        username: messageUsernameLabel(authorProfile),
        timestamp: timeAgo,
        nameColor: context.colors.onSurface,
        metadataColor: context.colors.onSurfaceVariant,
        displayNameKey: ValueKey('search-message-author-${hit.eventId}'),
        usernameKey: ValueKey('search-message-username-${hit.eventId}'),
        timestampKey: ValueKey('search-message-timestamp-${hit.eventId}'),
      ),
      subtitle: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const SizedBox(height: 2),
          Row(
            key: ValueKey('search-message-context-${hit.eventId}'),
            children: [
              Flexible(
                child: Text(
                  isDm
                      ? 'Direct message'
                      : hasChannelName
                      ? 'Message in'
                      : 'Message',
                  style: activityContextTextStyle.copyWith(
                    color: context.colors.onSurfaceVariant,
                  ),
                  overflow: TextOverflow.ellipsis,
                ),
              ),
              if (!isDm && hasChannelName) ...[
                const SizedBox(width: Grid.half),
                Flexible(
                  child: Container(
                    padding: const EdgeInsets.symmetric(
                      horizontal: Grid.half + Grid.quarter,
                      vertical: Grid.quarter / 2,
                    ),
                    decoration: BoxDecoration(
                      color: context.colors.surfaceContainerHighest,
                      borderRadius: BorderRadius.circular(Radii.xs),
                    ),
                    child: Text(
                      '#$channelName',
                      key: ValueKey('search-message-channel-${hit.eventId}'),
                      style: activityContextTextStyle.copyWith(
                        color: context.colors.onSurfaceVariant,
                      ),
                      overflow: TextOverflow.ellipsis,
                    ),
                  ),
                ),
              ],
            ],
          ),
          const SizedBox(height: Grid.half),
          MessageContent(
            key: ValueKey('search-message-body-${hit.eventId}'),
            content: hit.content,
            mentionNames: mentionNames,
            mentionLabels: labels,
            agentMentionPubkeys: agentMentionPubkeys,
            tags: hit.tags,
            maxLines: 2,
            baseStyle: activityPreviewTextStyle.copyWith(
              color: context.colors.onSurface,
            ),
          ),
        ],
      ),
      onTap: () {
        onResultSelected();
        _navigateToHit(context, hit, channel);
      },
    );
  }

  void _navigateToHit(BuildContext context, SearchHit hit, Channel? channel) {
    if (channel == null) return;

    if (hit.kind == 45001) {
      Navigator.of(context).push(
        MaterialPageRoute<void>(
          builder: (_) => ForumThreadPage(
            channelId: channel.id,
            postEventId: hit.eventId,
            currentPubkey: currentPubkey,
            isMember: channel.isMember,
            isArchived: channel.isArchived,
          ),
        ),
      );
    } else {
      Navigator.of(context).push(
        MaterialPageRoute<void>(
          builder: (_) => ChannelDetailPage(channel: channel),
        ),
      );
    }
  }
}

class _SectionLabel extends StatelessWidget {
  final String label;

  const _SectionLabel({required this.label});

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(
        Grid.gutter,
        Grid.xs,
        Grid.gutter,
        Grid.half,
      ),
      child: Text(
        label,
        key: ValueKey('search-section-${label.toLowerCase()}'),
        style: activityContextTextStyle.copyWith(
          color: context.colors.onSurfaceVariant,
        ),
      ),
    );
  }
}
