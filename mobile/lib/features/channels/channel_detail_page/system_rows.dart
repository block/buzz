part of '../channel_detail_page.dart';

class _SystemMessageRow extends HookConsumerWidget {
  final TimelineMessage message;
  final List<TimelineMessage>? groupedMessages;
  final String channelId;
  final String? currentPubkey;
  final List<TimelineMessage>? allMessages;
  final bool isMember;
  final bool isArchived;

  const _SystemMessageRow({
    required this.message,
    this.groupedMessages,
    required this.channelId,
    this.currentPubkey,
    this.allMessages,
    this.isMember = false,
    this.isArchived = false,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final spotlightKey = useMemoized(() => GlobalKey());
    final systemEvent = message.systemEvent;
    if (systemEvent == null) {
      return const SizedBox.shrink();
    }

    final userCache = ref.watch(userCacheProvider);
    final sourceMessages = groupedMessages ?? [message];
    final groupedMembership = _membershipDisplayEvent(sourceMessages);

    final identityNames = ref.watch(channelIdentityNamesProvider(channelId));
    String resolveLabel(String? pubkey) {
      if (pubkey == null) return 'Someone';
      if (userCache[pubkey.toLowerCase()] == null) {
        ref.read(userCacheProvider.notifier).get(pubkey.toLowerCase());
      }
      return identityNames.labelFor(pubkey);
    }

    final target = systemEvent.targetPubkey?.trim().toLowerCase();
    final messageStyleAction = switch (systemEvent.type) {
      SystemEventType.memberRemoved => <InlineSpan>[
        const TextSpan(text: 'removed '),
        if (target == null || target.isEmpty)
          TextSpan(text: resolveLabel(target))
        else
          _systemNameSpan(context, target, channelId, resolveLabel),
        const TextSpan(text: ' from the channel'),
      ],
      SystemEventType.topicChanged => [
        TextSpan(
          text: describeChannelTextFieldChange('topic', systemEvent.topic),
        ),
      ],
      SystemEventType.purposeChanged => [
        TextSpan(
          text: describeChannelTextFieldChange('purpose', systemEvent.purpose),
        ),
      ],
      SystemEventType.memberLeft => [const TextSpan(text: 'left the channel')],
      SystemEventType.channelArchived => [
        const TextSpan(text: 'archived this channel'),
      ],
      SystemEventType.channelUnarchived => [
        const TextSpan(text: 'unarchived this channel'),
      ],
      SystemEventType.channelCreated => [
        const TextSpan(text: 'created this channel'),
      ],
      SystemEventType.huddleStarted => [
        const TextSpan(text: 'started a huddle'),
      ],
      SystemEventType.huddleEnded => [const TextSpan(text: 'ended the huddle')],
      _ => null,
    };
    final messageStyleActor = messageStyleAction == null
        ? null
        : systemEvent.actorPubkey?.trim();
    final reactions = groupedMessages == null
        ? message.reactions
        : _aggregateSystemMessageReactions(sourceMessages);

    void toggleGroupedReaction(String emoji) {
      final actions = ref.read(channelActionsProvider);
      final reactedMessages = sourceMessages.where(
        (source) => source.reactions.any(
          (reaction) =>
              reaction.emoji == emoji &&
              reaction.reactedByCurrentUser &&
              reaction.currentUserReactionId != null,
        ),
      );
      if (reactedMessages.isEmpty) {
        actions.addReaction(message.id, emoji);
        return;
      }
      for (final source in reactedMessages) {
        final reaction = source.reactions.firstWhere(
          (candidate) =>
              candidate.emoji == emoji &&
              candidate.reactedByCurrentUser &&
              candidate.currentUserReactionId != null,
        );
        actions.removeReaction(reaction.currentUserReactionId!, emoji);
      }
    }

    void openReactionPopover(MessageLongPressDetails details) {
      showMessageActions(
        context: context,
        ref: ref,
        message: message,
        reactions: reactions,
        onToggleReaction: toggleGroupedReaction,
        channelId: channelId,
        canManageMessage: false,
        allMessages: null,
        currentPubkey: currentPubkey,
        isMember: isMember,
        isArchived: isArchived,
        anchorRect: details.anchorRect,
        captureAnchorSnapshot: details.captureSnapshot,
        popoverSpotlightPadding: EdgeInsets.fromLTRB(
          Grid.xxs,
          Grid.xxs,
          Grid.xxs,
          reactions.isEmpty ? Grid.xxs : Grid.quarter,
        ),
      );
    }

    final isHuddle =
        systemEvent.type == SystemEventType.huddleStarted &&
        systemEvent.ephemeralChannelId != null;
    final actor = (systemEvent.actorPubkey ?? message.pubkey).toLowerCase();
    Widget withActor(Widget content) {
      if (!isHuddle) return content;
      return Row(
        key: ValueKey('huddle-activity-${message.id}'),
        crossAxisAlignment: CrossAxisAlignment.end,
        children: [
          Semantics(
            container: true,
            label: 'Open ${resolveLabel(actor)} profile',
            button: true,
            excludeSemantics: true,
            onTap: () => showUserProfileSheet(
              context,
              actor,
              names: channelIdentityNamesProvider(channelId),
            ),
            child: GestureDetector(
              excludeFromSemantics: true,
              onTap: () => showUserProfileSheet(
                context,
                actor,
                names: channelIdentityNamesProvider(channelId),
              ),
              child: _UserAvatar(
                profile: userCache[actor],
                pubkey: actor,
                isAgent: userCache[actor]?.ownerPubkey != null,
              ),
            ),
          ),
          const SizedBox(width: messageAvatarContentGap),
          Expanded(child: content),
        ],
      );
    }

    return Material(
      color: Colors.transparent,
      borderRadius: BorderRadius.circular(Radii.md),
      clipBehavior: Clip.antiAlias,
      child: MessageLongPressInkWell(
        key: ValueKey('system-message-row-${message.id}'),
        onLongPressDetails: openReactionPopover,
        snapshotKey: spotlightKey,
        borderRadius: BorderRadius.circular(Radii.md),
        highlightColor: context.colors.primary.withValues(alpha: 0.1),
        child: Padding(
          padding: const EdgeInsets.symmetric(vertical: Grid.xxs),
          child: Column(
            crossAxisAlignment: isHuddle
                ? CrossAxisAlignment.start
                : CrossAxisAlignment.center,
            children: [
              if (!isHuddle)
                RepaintBoundary(
                  key: spotlightKey,
                  child: Align(
                    alignment: isHuddle
                        ? Alignment.centerLeft
                        : Alignment.center,
                    child: Container(
                      key: ValueKey('activity-card-${message.id}'),
                      padding: const EdgeInsets.symmetric(
                        horizontal: Grid.twelve,
                        vertical: Grid.xxs,
                      ),
                      decoration: BoxDecoration(
                        color: context.colors.onSurface.withValues(alpha: 0.06),
                        borderRadius: BorderRadius.circular(Radii.container),
                      ),
                      child: IntrinsicWidth(
                        child: groupedMembership != null
                            ? _MembershipSystemMessageContent(
                                event: groupedMembership,
                                channelId: channelId,
                                createdAt: message.createdAt,
                                resolveLabel: resolveLabel,
                                userCache: userCache,
                              )
                            : messageStyleActor != null &&
                                  messageStyleActor.isNotEmpty &&
                                  messageStyleAction != null
                            ? _MessageStyleSystemMessageContent(
                                displayPubkey: messageStyleActor,
                                channelId: channelId,
                                createdAt: message.createdAt,
                                resolveLabel: resolveLabel,
                                userCache: userCache,
                                actionSpans: messageStyleAction,
                              )
                            : Text.rich(
                                TextSpan(
                                  children: [
                                    TextSpan(
                                      text: systemEvent.describe(resolveLabel),
                                    ),
                                  ],
                                ),
                                textAlign: TextAlign.center,
                                style: _systemActionTextStyle(context),
                              ),
                      ),
                    ),
                  ),
                ),
              if (systemEvent.type == SystemEventType.huddleStarted &&
                  systemEvent.ephemeralChannelId != null)
                MessageFooterLayout(
                  animateSize: true,
                  overlap: reactions.isNotEmpty ? Grid.xs : 0,
                  footer: reactions.isEmpty
                      ? const SizedBox.shrink()
                      : Padding(
                          padding: const EdgeInsets.only(
                            left:
                                messageAvatarSize +
                                messageAvatarContentGap +
                                Grid.xxs,
                          ),
                          child: ReactionRow(
                            combined: true,
                            messageId: message.id,
                            channelId: channelId,
                            reactions: reactions,
                            onToggle: (emoji) =>
                                toggleReaction(ref, message, emoji),
                            showAddButton: isMember && !isArchived,
                            onAddReaction: () => showAddReactionPicker(
                              context: context,
                              ref: ref,
                              message: message,
                            ),
                          ),
                        ),
                  child: withActor(
                    RepaintBoundary(
                      key: spotlightKey,
                      child: _HuddleJoinSurface(
                        message: message,
                        allMessages: allMessages ?? const [],
                        parentChannelId: channelId,
                        isMember: isMember,
                        isArchived: isArchived,
                      ),
                    ),
                  ),
                ),
              if (!isHuddle && reactions.isNotEmpty)
                IntrinsicWidth(
                  child: ReactionRow(
                    messageId: message.id,
                    channelId: channelId,
                    reactions: reactions,
                    onToggle: groupedMessages == null
                        ? (emoji) => toggleReaction(ref, message, emoji)
                        : toggleGroupedReaction,
                  ),
                ),
            ],
          ),
        ),
      ),
    );
  }
}

class _MembershipDisplayEvent {
  final String? actorPubkey;
  final List<String> targetPubkeys;
  final bool isSelfJoin;
  final bool isRemoval;

  const _MembershipDisplayEvent({
    required this.actorPubkey,
    required this.targetPubkeys,
    required this.isSelfJoin,
    required this.isRemoval,
  });
}

_MembershipDisplayEvent? _membershipDisplayEvent(
  List<TimelineMessage> messages,
) {
  if (messages.isEmpty) return null;

  final first = messages.first.systemEvent;
  final firstActor = first?.actorPubkey?.trim().toLowerCase();
  final firstTarget = first?.targetPubkey?.trim().toLowerCase();
  if ((first?.type != SystemEventType.memberJoined &&
          first?.type != SystemEventType.memberRemoved) ||
      firstActor == null ||
      firstActor.isEmpty ||
      firstTarget == null ||
      firstTarget.isEmpty) {
    return null;
  }

  final isRemoval = first!.type == SystemEventType.memberRemoved;
  final isSelfJoin = !isRemoval && firstActor == firstTarget;
  final targets = <String>[];
  for (final message in messages) {
    final event = message.systemEvent;
    final actor = event?.actorPubkey?.trim().toLowerCase();
    final target = event?.targetPubkey?.trim().toLowerCase();
    if (event?.type != first.type ||
        actor == null ||
        actor.isEmpty ||
        target == null ||
        target.isEmpty ||
        (isSelfJoin
            ? actor != target
            : actor != firstActor || (!isRemoval && actor == target))) {
      return null;
    }
    targets.add(target);
  }

  return _MembershipDisplayEvent(
    actorPubkey: isSelfJoin ? null : firstActor,
    targetPubkeys: targets.toSet().toList(),
    isSelfJoin: isSelfJoin,
    isRemoval: isRemoval,
  );
}

List<TimelineReaction> _aggregateSystemMessageReactions(
  List<TimelineMessage> messages,
) {
  final byEmoji = <String, List<TimelineReaction>>{};
  for (final message in messages) {
    for (final reaction in message.reactions) {
      byEmoji.putIfAbsent(reaction.emoji, () => []).add(reaction);
    }
  }

  return [
    for (final entry in byEmoji.entries)
      () {
        final reactions = entry.value;
        final userPubkeys = {
          for (final reaction in reactions) ...reaction.userPubkeys,
        }.toList();
        final reactedByCurrentUser = reactions.any(
          (reaction) => reaction.reactedByCurrentUser,
        );
        final currentUserReactionId = reactions
            .where((reaction) => reaction.currentUserReactionId != null)
            .firstOrNull
            ?.currentUserReactionId;
        final fallbackCount = reactions.fold<int>(
          0,
          (total, reaction) => total + reaction.count,
        );
        return TimelineReaction(
          emoji: entry.key,
          count: userPubkeys.isEmpty ? fallbackCount : userPubkeys.length,
          reactedByCurrentUser: reactedByCurrentUser,
          userPubkeys: userPubkeys,
          emojiUrl: reactions.first.emojiUrl,
          currentUserReactionId: currentUserReactionId,
        );
      }(),
  ];
}

class _MembershipSystemMessageContent extends StatelessWidget {
  final _MembershipDisplayEvent event;
  final String channelId;
  final int createdAt;
  final String Function(String? pubkey) resolveLabel;
  final Map<String, UserProfile> userCache;

  const _MembershipSystemMessageContent({
    required this.event,
    required this.channelId,
    required this.createdAt,
    required this.resolveLabel,
    required this.userCache,
  });

  @override
  Widget build(BuildContext context) {
    final firstTarget = event.targetPubkeys.first;
    final additionalTargets = event.targetPubkeys.skip(1).take(1).toList();
    final hiddenCount = event.targetPubkeys.length - 2;
    final actionSpans = <InlineSpan>[
      TextSpan(
        text: event.isSelfJoin
            ? 'joined the channel'
            : event.isRemoval
            ? 'removed '
            : 'added ',
      ),
      if (!event.isSelfJoin)
        _systemNameSpan(context, firstTarget, channelId, resolveLabel),
      if (additionalTargets.isNotEmpty)
        TextSpan(
          text: event.isSelfJoin
              ? ' along with '
              : additionalTargets.length == 1 && hiddenCount <= 0
              ? ' and '
              : ', ',
        ),
      ..._memberNameSpans(
        context,
        targets: additionalTargets,
        channelId: channelId,
        resolveLabel: resolveLabel,
      ),
      if (hiddenCount > 0)
        TextSpan(
          text: ' and $hiddenCount ${hiddenCount == 1 ? 'other' : 'others'}',
        ),
      if (!event.isSelfJoin)
        TextSpan(
          text: event.isRemoval ? ' from the channel' : ' to the channel',
        ),
      if (hiddenCount > 0) ...[
        const TextSpan(text: '. '),
        WidgetSpan(
          alignment: PlaceholderAlignment.baseline,
          baseline: TextBaseline.alphabetic,
          child: Semantics(
            button: true,
            child: GestureDetector(
              onTap: () => showBuzzModalBottomSheet<void>(
                context: context,
                title: event.isRemoval ? 'Removed members' : 'Added members',
                isScrollControlled: true,
                showDragHandle: true,
                builder: (_) => _ActivityMembersSheet(
                  pubkeys: event.targetPubkeys,
                  channelId: channelId,
                ),
              ),
              child: Text(
                'See all',
                style: _systemActionTextStyle(
                  context,
                )?.copyWith(fontWeight: FontWeight.w600),
              ),
            ),
          ),
        ),
      ],
    ];

    return _MessageStyleSystemMessageContent(
      displayPubkey: event.isSelfJoin ? firstTarget : event.actorPubkey!,
      channelId: channelId,
      createdAt: createdAt,
      resolveLabel: resolveLabel,
      userCache: userCache,
      actionSpans: actionSpans,
    );
  }
}

TextStyle? _systemActionTextStyle(BuildContext context) {
  return bodyExtraSmallTextStyle.copyWith(
    color: context.colors.onSurfaceVariant,
  );
}

class _MessageStyleSystemMessageContent extends StatelessWidget {
  final String displayPubkey;
  final String channelId;
  final int createdAt;
  final String Function(String? pubkey) resolveLabel;
  final Map<String, UserProfile> userCache;
  final List<InlineSpan> actionSpans;

  const _MessageStyleSystemMessageContent({
    required this.displayPubkey,
    required this.channelId,
    required this.createdAt,
    required this.resolveLabel,
    required this.userCache,
    required this.actionSpans,
  });

  @override
  Widget build(BuildContext context) {
    return Text.rich(
      TextSpan(
        children: [
          _systemNameSpan(context, displayPubkey, channelId, resolveLabel),
          const TextSpan(text: ' '),
          ...actionSpans,
        ],
      ),
      textAlign: TextAlign.center,
      style: _systemActionTextStyle(context),
    );
  }
}

WidgetSpan _systemNameSpan(
  BuildContext context,
  String pubkey,
  String channelId,
  String Function(String?) resolveLabel,
) {
  return WidgetSpan(
    alignment: PlaceholderAlignment.baseline,
    baseline: TextBaseline.alphabetic,
    child: Semantics(
      button: true,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: () => showUserProfileSheet(
          context,
          pubkey,
          names: channelIdentityNamesProvider(channelId),
        ),
        child: ConstrainedBox(
          constraints: BoxConstraints(
            maxWidth: max(
              0,
              MediaQuery.sizeOf(context).width - Grid.gutter * 2 - Grid.lg,
            ),
          ),
          child: Text(
            resolveLabel(pubkey),
            key: ValueKey('system-message-author-$pubkey'),
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: _systemActionTextStyle(context),
          ),
        ),
      ),
    ),
  );
}

List<InlineSpan> _memberNameSpans(
  BuildContext context, {
  required List<String> targets,
  required String channelId,
  required String Function(String? pubkey) resolveLabel,
}) {
  return [
    for (var index = 0; index < targets.length; index++) ...[
      if (index > 0)
        TextSpan(
          text: index == targets.length - 1
              ? (targets.length == 2 ? ' and ' : ', and ')
              : ', ',
        ),
      _systemNameSpan(context, targets[index], channelId, resolveLabel),
    ],
  ];
}

class _ThreadSummaryRow extends ConsumerWidget {
  final ThreadSummary summary;
  final TimelineMessage message;
  final List<TimelineMessage> allMessages;
  final String channelId;
  final String? currentPubkey;
  final bool isMember;
  final bool isArchived;

  const _ThreadSummaryRow({
    required this.summary,
    required this.message,
    required this.allMessages,
    required this.channelId,
    required this.currentPubkey,
    required this.isMember,
    required this.isArchived,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    return ReplySummaryLink(
      key: ValueKey('thread-summary-${message.id}'),
      summary: summary,
      hasReactions: message.reactions.isNotEmpty,
      onTap: () {
        Navigator.of(context).push(
          MaterialPageRoute<void>(
            builder: (_) => ThreadDetailPage(
              threadHead: message,
              allMessages: allMessages,
              channelId: channelId,
              currentPubkey: currentPubkey,
              isMember: isMember,
              isArchived: isArchived,
            ),
          ),
        );
      },
    );
  }
}
