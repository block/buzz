part of '../channel_detail_page.dart';

class _SystemMessageRow extends ConsumerWidget {
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
    final systemEvent = message.systemEvent;
    if (systemEvent == null) return const SizedBox.shrink();

    final userCache = ref.watch(userCacheProvider);
    final sourceMessages = groupedMessages ?? [message];
    final groupedMembership = _membershipDisplayEvent(sourceMessages);
    final channelCreator = systemEvent.type == SystemEventType.channelCreated
        ? systemEvent.actorPubkey?.trim()
        : null;

    String resolveLabel(String? pubkey) {
      if (pubkey == null) return 'Someone';
      final profile =
          userCache[pubkey.toLowerCase()] ??
          ref.read(userCacheProvider.notifier).get(pubkey.toLowerCase());
      return profile?.label ?? shortPubkey(pubkey);
    }

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

    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onLongPress: () => showMessageActions(
        context: context,
        ref: ref,
        message: message,
        channelId: channelId,
        canManageMessage: false,
        allMessages: null,
        currentPubkey: currentPubkey,
        isMember: isMember,
        isArchived: isArchived,
      ),
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: Grid.xxs),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            if (groupedMembership != null)
              _MembershipSystemMessageContent(
                event: groupedMembership,
                createdAt: message.createdAt,
                resolveLabel: resolveLabel,
                userCache: userCache,
              )
            else if (channelCreator != null && channelCreator.isNotEmpty)
              _MessageStyleSystemMessageContent(
                displayPubkey: channelCreator,
                createdAt: message.createdAt,
                resolveLabel: resolveLabel,
                userCache: userCache,
                actionSpans: const [TextSpan(text: 'created this channel')],
              )
            else
              Row(
                children: [
                  _systemEventAvatar(context, systemEvent, userCache),
                  const SizedBox(width: Grid.xxs),
                  Expanded(
                    child: Text(
                      systemEvent.describe(resolveLabel),
                      style: context.textTheme.bodyLarge?.copyWith(
                        color: context.colors.onSurfaceVariant,
                      ),
                    ),
                  ),
                  _messageTimestamp(context, message.createdAt),
                ],
              ),
            if (reactions.isNotEmpty)
              Padding(
                padding: const EdgeInsets.only(left: 36 + Grid.xxs),
                child: ReactionRow(
                  reactions: reactions,
                  onToggle: groupedMessages == null
                      ? (emoji) => toggleReaction(ref, message, emoji)
                      : toggleGroupedReaction,
                ),
              ),
          ],
        ),
      ),
    );
  }
}

const _maxVisibleAdditionalMemberNames = 3;

class _MembershipDisplayEvent {
  final String? actorPubkey;
  final List<String> targetPubkeys;
  final bool isSelfJoin;

  const _MembershipDisplayEvent({
    required this.actorPubkey,
    required this.targetPubkeys,
    required this.isSelfJoin,
  });
}

_MembershipDisplayEvent? _membershipDisplayEvent(
  List<TimelineMessage> messages,
) {
  if (messages.isEmpty) return null;

  final first = messages.first.systemEvent;
  final firstActor = first?.actorPubkey?.trim().toLowerCase();
  final firstTarget = first?.targetPubkey?.trim().toLowerCase();
  if (first?.type != SystemEventType.memberJoined ||
      firstActor == null ||
      firstActor.isEmpty ||
      firstTarget == null ||
      firstTarget.isEmpty) {
    return null;
  }

  final isSelfJoin = firstActor == firstTarget;
  final targets = <String>[];
  for (final message in messages) {
    final event = message.systemEvent;
    final actor = event?.actorPubkey?.trim().toLowerCase();
    final target = event?.targetPubkey?.trim().toLowerCase();
    if (event?.type != SystemEventType.memberJoined ||
        actor == null ||
        actor.isEmpty ||
        target == null ||
        target.isEmpty ||
        (isSelfJoin
            ? actor != target
            : actor != firstActor || actor == target)) {
      return null;
    }
    targets.add(target);
  }

  return _MembershipDisplayEvent(
    actorPubkey: isSelfJoin ? null : firstActor,
    targetPubkeys: targets,
    isSelfJoin: isSelfJoin,
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
  final int createdAt;
  final String Function(String? pubkey) resolveLabel;
  final Map<String, UserProfile> userCache;

  const _MembershipSystemMessageContent({
    required this.event,
    required this.createdAt,
    required this.resolveLabel,
    required this.userCache,
  });

  @override
  Widget build(BuildContext context) {
    final firstTarget = event.targetPubkeys.first;
    final additionalTargets = event.targetPubkeys.skip(1).toList();
    final visibleTargets = additionalTargets
        .take(_maxVisibleAdditionalMemberNames)
        .toList();
    final hiddenTargets = additionalTargets
        .skip(_maxVisibleAdditionalMemberNames)
        .toList();
    final actionStyle = _systemActionTextStyle(context);
    final actionSpans = <InlineSpan>[
      TextSpan(
        text: event.isSelfJoin
            ? 'joined the channel'
            : 'was added by ${resolveLabel(event.actorPubkey)}',
      ),
      if (additionalTargets.isNotEmpty)
        TextSpan(text: event.isSelfJoin ? ' along with ' : ', along with '),
      ..._memberNameSpans(
        context,
        visibleTargets: visibleTargets,
        hiddenTargets: hiddenTargets,
        resolveLabel: resolveLabel,
        style: actionStyle,
      ),
    ];

    return _MessageStyleSystemMessageContent(
      displayPubkey: firstTarget,
      createdAt: createdAt,
      resolveLabel: resolveLabel,
      userCache: userCache,
      actionSpans: actionSpans,
    );
  }
}

TextStyle? _systemActionTextStyle(BuildContext context) {
  return context.textTheme.bodyLarge?.copyWith(
    color: context.colors.onSurfaceVariant,
  );
}

class _MessageStyleSystemMessageContent extends StatelessWidget {
  final String displayPubkey;
  final int createdAt;
  final String Function(String? pubkey) resolveLabel;
  final Map<String, UserProfile> userCache;
  final List<InlineSpan> actionSpans;

  const _MessageStyleSystemMessageContent({
    required this.displayPubkey,
    required this.createdAt,
    required this.resolveLabel,
    required this.userCache,
    required this.actionSpans,
  });

  @override
  Widget build(BuildContext context) {
    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        _UserAvatar(
          profile: userCache[displayPubkey.toLowerCase()],
          pubkey: displayPubkey,
          size: 36,
        ),
        const SizedBox(width: Grid.xxs),
        Expanded(
          child: Transform.translate(
            offset: const Offset(0, -Grid.quarter),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Row(
                  crossAxisAlignment: CrossAxisAlignment.center,
                  children: [
                    Text(
                      resolveLabel(displayPubkey),
                      style: context.textTheme.titleSmall?.copyWith(
                        color: context.colors.onSurface,
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                    const SizedBox(width: Grid.xxs),
                    _messageTimestamp(context, createdAt),
                  ],
                ),
                Text.rich(
                  TextSpan(
                    style: _systemActionTextStyle(context),
                    children: actionSpans,
                  ),
                ),
              ],
            ),
          ),
        ),
      ],
    );
  }
}

List<InlineSpan> _memberNameSpans(
  BuildContext context, {
  required List<String> visibleTargets,
  required List<String> hiddenTargets,
  required String Function(String? pubkey) resolveLabel,
  required TextStyle? style,
}) {
  final spans = <InlineSpan>[];
  for (var index = 0; index < visibleTargets.length; index++) {
    final isLast = index == visibleTargets.length - 1;
    final separator = index == 0
        ? ''
        : isLast && hiddenTargets.isEmpty
        ? (visibleTargets.length == 2 ? ' and ' : ', and ')
        : ', ';
    spans.add(
      TextSpan(text: '$separator${resolveLabel(visibleTargets[index])}'),
    );
  }

  if (hiddenTargets.isNotEmpty) {
    final hiddenLabels = hiddenTargets.map(resolveLabel).toList();
    spans
      ..add(const TextSpan(text: ', and '))
      ..add(
        WidgetSpan(
          alignment: PlaceholderAlignment.baseline,
          baseline: TextBaseline.alphabetic,
          child: Tooltip(
            message: hiddenLabels.join('\n'),
            triggerMode: TooltipTriggerMode.tap,
            child: Text(
              '${hiddenTargets.length} others',
              key: const Key('membership-overflow'),
              style: style?.copyWith(
                decoration: TextDecoration.underline,
                decorationStyle: TextDecorationStyle.dotted,
              ),
            ),
          ),
        ),
      );
  }

  return spans;
}

Widget _systemEventAvatar(
  BuildContext context,
  SystemEvent event,
  Map<String, UserProfile> userCache,
) {
  final hasTarget =
      event.targetPubkey != null && event.targetPubkey != event.actorPubkey;

  if (event.actorPubkey != null && hasTarget) {
    // Two-avatar stack: actor + target (e.g. "Alice added Bob").
    return SizedBox(
      width: 32,
      height: 20,
      child: Stack(
        children: [
          SmallAvatar(pubkey: event.actorPubkey!, userCache: userCache),
          Positioned(
            left: 12,
            child: SmallAvatar(
              pubkey: event.targetPubkey!,
              userCache: userCache,
            ),
          ),
        ],
      ),
    );
  }

  if (event.actorPubkey != null) {
    return SmallAvatar(pubkey: event.actorPubkey!, userCache: userCache);
  }

  // Fallback: generic icon when no actor is available.
  return Container(
    width: 20,
    height: 20,
    decoration: BoxDecoration(
      color: context.colors.surfaceContainerHighest,
      shape: BoxShape.circle,
    ),
    child: Icon(
      LucideIcons.arrowLeftRight,
      size: 12,
      color: context.colors.onSurfaceVariant,
    ),
  );
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
    final userCache = ref.watch(userCacheProvider);

    return GestureDetector(
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
      child: Padding(
        padding: const EdgeInsets.only(
          left: 36 + Grid.xxs,
          top: Grid.half,
          bottom: Grid.half,
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            // Stacked participant avatars.
            SizedBox(
              width: 32.0 + (summary.participantPubkeys.length - 1) * 20.0,
              height: 32,
              child: Stack(
                children: [
                  for (var i = 0; i < summary.participantPubkeys.length; i++)
                    Positioned(
                      left: i * 20.0,
                      child: SmallAvatar(
                        pubkey: summary.participantPubkeys[i],
                        userCache: userCache,
                        size: 32,
                      ),
                    ),
                ],
              ),
            ),
            const SizedBox(width: Grid.xxs),
            Text.rich(
              TextSpan(
                children: [
                  TextSpan(
                    text:
                        '${summary.replyCount} ${summary.replyCount == 1 ? 'reply' : 'replies'}',
                    style: context.textTheme.labelMedium?.copyWith(
                      color: context.colors.primary,
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                  if (summary.lastReplyAt case final lastReplyAt?) ...[
                    TextSpan(
                      text: ' · ',
                      style: context.textTheme.labelMedium?.copyWith(
                        color: context.colors.onSurfaceVariant.withValues(
                          alpha: 0.5,
                        ),
                      ),
                    ),
                    TextSpan(
                      text:
                          'last reply ${formatThreadSummaryLastReplyTime(lastReplyAt)}',
                      style: context.textTheme.labelMedium?.copyWith(
                        color: context.colors.onSurfaceVariant,
                        fontWeight: FontWeight.w400,
                      ),
                    ),
                  ],
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}
