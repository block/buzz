part of '../channel_detail_page.dart';

class _SystemMessageRow extends HookConsumerWidget {
  final TimelineMessage message;
  final List<TimelineMessage>? groupedMessages;
  final String channelId;
  final String? currentPubkey;
  final List<TimelineMessage>? allMessages;
  final bool isMember;
  final bool isArchived;
  final bool startsDay;
  final bool endsDay;

  const _SystemMessageRow({
    required this.message,
    this.groupedMessages,
    required this.channelId,
    this.currentPubkey,
    this.allMessages,
    this.isMember = false,
    this.isArchived = false,
    this.startsDay = false,
    this.endsDay = false,
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
        actions.addReaction(message.id, emoji, targetAuthor: message.pubkey);
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
    final ownActor = currentPubkey?.toLowerCase() == actor;
    final ownHuddle =
        ownActor &&
        (isHuddle || systemEvent.type == SystemEventType.huddleEnded);
    Widget withActor(Widget content) {
      if (!isHuddle) return content;
      return Row(
        key: ValueKey('huddle-activity-${message.id}'),
        textDirection: ownHuddle ? TextDirection.rtl : TextDirection.ltr,
        crossAxisAlignment: CrossAxisAlignment.end,
        children: [
          if (!ownHuddle)
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
          if (!ownHuddle) const SizedBox(width: messageAvatarContentGap),
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
          padding: EdgeInsets.only(
            top: startsDay ? 0 : Grid.xxs,
            bottom: endsDay ? 0 : Grid.xxs,
          ),
          child: Column(
            crossAxisAlignment: ownHuddle
                ? CrossAxisAlignment.end
                : isHuddle
                ? CrossAxisAlignment.start
                : CrossAxisAlignment.center,
            children: [
              if (!isHuddle)
                RepaintBoundary(
                  key: spotlightKey,
                  child: Align(
                    alignment: ownHuddle
                        ? Alignment.centerRight
                        : isHuddle
                        ? Alignment.centerLeft
                        : Alignment.center,
                    child: Padding(
                      key: ValueKey('activity-card-${message.id}'),
                      padding: const EdgeInsets.symmetric(
                        horizontal: Grid.twelve,
                        vertical: Grid.xxs,
                      ),
                      child: IntrinsicWidth(
                        child: groupedMembership != null
                            ? _MembershipSystemMessageContent(
                                event: groupedMembership,
                                channelId: channelId,
                                resolveLabel: resolveLabel,
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
                      : Align(
                          alignment: ownHuddle
                              ? Alignment.centerRight
                              : Alignment.centerLeft,
                          child: Padding(
                            padding: EdgeInsets.only(
                              left: ownHuddle
                                  ? 0
                                  : messageAvatarSize +
                                        messageAvatarContentGap +
                                        Grid.xxs,
                              right: ownHuddle ? Grid.xxs : 0,
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

enum _MembershipDisplayKind {
  /// One person added everyone.
  added,

  /// Several people added members.
  addedByMany,

  /// One person removed everyone.
  removed,

  /// Additions and removals together, or removals by several people.
  mixed,
}

class _MembershipDisplayEvent {
  final _MembershipDisplayKind kind;

  /// The adder or remover for [_MembershipDisplayKind.added] and
  /// [_MembershipDisplayKind.removed]; null otherwise.
  final String? actorPubkey;

  /// People whose latest change in the group is an addition.
  final List<String> addedPubkeys;

  /// People whose latest change in the group is a removal.
  final List<String> removedPubkeys;

  const _MembershipDisplayEvent({
    required this.kind,
    required this.actorPubkey,
    required this.addedPubkeys,
    required this.removedPubkeys,
  });
}

/// Describes a run of additions (by someone else) and removals. Like
/// desktop, each person appears once, under their latest change. Returns
/// null for anything else, such as a self-join.
_MembershipDisplayEvent? _membershipDisplayEvent(
  List<TimelineMessage> messages,
) {
  if (messages.isEmpty) return null;

  final latest = <String, ({SystemEventType type, String actor})>{};
  for (final message in messages) {
    final event = message.systemEvent;
    final actor = event?.actorPubkey?.trim().toLowerCase();
    final target = event?.targetPubkey?.trim().toLowerCase();
    if (event == null ||
        (event.type != SystemEventType.memberJoined &&
            event.type != SystemEventType.memberRemoved) ||
        actor == null ||
        actor.isEmpty ||
        target == null ||
        target.isEmpty ||
        (event.type == SystemEventType.memberJoined && actor == target)) {
      return null;
    }
    latest[target] = (type: event.type, actor: actor);
  }

  final added = <String>[];
  final removed = <String>[];
  final adders = <String>{};
  final removers = <String>{};
  latest.forEach((target, change) {
    if (change.type == SystemEventType.memberJoined) {
      added.add(target);
      adders.add(change.actor);
    } else {
      removed.add(target);
      removers.add(change.actor);
    }
  });

  final kind = removed.isEmpty
      ? adders.length == 1
            ? _MembershipDisplayKind.added
            : _MembershipDisplayKind.addedByMany
      : added.isEmpty && removers.length == 1
      ? _MembershipDisplayKind.removed
      : _MembershipDisplayKind.mixed;

  return _MembershipDisplayEvent(
    kind: kind,
    actorPubkey: switch (kind) {
      _MembershipDisplayKind.added => adders.single,
      _MembershipDisplayKind.removed => removers.single,
      _ => null,
    },
    addedPubkeys: added,
    removedPubkeys: removed,
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
  final String Function(String? pubkey) resolveLabel;

  const _MembershipSystemMessageContent({
    required this.event,
    required this.channelId,
    required this.resolveLabel,
  });

  @override
  Widget build(BuildContext context) {
    InlineSpan name(String pubkey) =>
        _systemNameSpan(context, pubkey, channelId, resolveLabel);

    final spans = switch (event.kind) {
      _MembershipDisplayKind.added || _MembershipDisplayKind.removed => () {
        final removed = event.kind == _MembershipDisplayKind.removed;
        final targets = removed ? event.removedPubkeys : event.addedPubkeys;
        return <InlineSpan>[
          name(event.actorPubkey!),
          TextSpan(text: removed ? ' removed ' : ' added '),
          ..._leadingNames(context, targets, name),
          TextSpan(text: removed ? ' from the channel' : ' to the channel'),
          ..._seeAll(
            context,
            targets,
            removed ? 'Removed members' : 'Added members',
          ),
        ];
      }(),
      _MembershipDisplayKind.addedByMany => <InlineSpan>[
        ..._leadingNames(context, event.addedPubkeys, name),
        const TextSpan(text: ' were added to the channel'),
        ..._seeAll(context, event.addedPubkeys, 'Added members'),
      ],
      _MembershipDisplayKind.mixed => <InlineSpan>[
        if (event.addedPubkeys.isNotEmpty)
          ..._summary(context, event.addedPubkeys, 'added', name),
        if (event.addedPubkeys.isNotEmpty && event.removedPubkeys.isNotEmpty)
          const TextSpan(text: ' · '),
        if (event.removedPubkeys.isNotEmpty)
          ..._summary(context, event.removedPubkeys, 'removed', name),
      ],
    };

    return Text.rich(
      TextSpan(children: spans),
      textAlign: TextAlign.center,
      style: _systemActionTextStyle(context),
    );
  }

  /// "Bob", "Bob and Carol", or "Bob, Carol and 3 others".
  List<InlineSpan> _leadingNames(
    BuildContext context,
    List<String> targets,
    InlineSpan Function(String pubkey) name,
  ) {
    final hiddenCount = targets.length - 2;
    return [
      name(targets.first),
      if (targets.length > 1) ...[
        TextSpan(text: hiddenCount > 0 ? ', ' : ' and '),
        name(targets[1]),
      ],
      if (hiddenCount > 0)
        TextSpan(
          text: ' and $hiddenCount ${hiddenCount == 1 ? 'other' : 'others'}',
        ),
    ];
  }

  List<InlineSpan> _seeAll(
    BuildContext context,
    List<String> targets,
    String title,
  ) {
    if (targets.length <= 2) return const [];
    return [
      const TextSpan(text: '. '),
      _membersSheetLink(context, 'See all', title, targets),
    ];
  }

  /// "Bob was added" or "Bob and 182 others were added", where the count
  /// opens the full list.
  List<InlineSpan> _summary(
    BuildContext context,
    List<String> targets,
    String action,
    InlineSpan Function(String pubkey) name,
  ) {
    final othersCount = targets.length - 1;
    return [
      name(targets.first),
      if (othersCount > 0) ...[
        const TextSpan(text: ' and '),
        _membersSheetLink(
          context,
          '$othersCount ${othersCount == 1 ? 'other' : 'others'}',
          action == 'added' ? 'Added members' : 'Removed members',
          targets,
        ),
      ],
      TextSpan(text: othersCount > 0 ? ' were $action' : ' was $action'),
    ];
  }

  WidgetSpan _membersSheetLink(
    BuildContext context,
    String label,
    String title,
    List<String> pubkeys,
  ) {
    return WidgetSpan(
      alignment: PlaceholderAlignment.baseline,
      baseline: TextBaseline.alphabetic,
      child: Semantics(
        button: true,
        child: GestureDetector(
          onTap: () => showBuzzModalBottomSheet<void>(
            context: context,
            title: title,
            isScrollControlled: true,
            showDragHandle: true,
            builder: (_) =>
                _ActivityMembersSheet(pubkeys: pubkeys, channelId: channelId),
          ),
          child: Text(
            label,
            style: _systemActionTextStyle(
              context,
            )?.copyWith(fontWeight: FontWeight.w600),
          ),
        ),
      ),
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

class _ThreadSummaryRow extends ConsumerWidget {
  final ThreadSummary? summary;

  /// Agents typing in this message's thread.
  final List<String> working;
  final TimelineMessage message;
  final List<TimelineMessage> allMessages;
  final String channelId;
  final String? currentPubkey;
  final bool isMember;
  final bool isArchived;

  const _ThreadSummaryRow({
    required this.summary,
    this.working = const [],
    required this.message,
    required this.allMessages,
    required this.channelId,
    required this.currentPubkey,
    required this.isMember,
    required this.isArchived,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final labels = watchChannelIdentityLabels(ref, channelId, working);
    return ReplySummaryLink(
      key: ValueKey('thread-summary-${message.id}'),
      mirrored: currentPubkey?.toLowerCase() == message.pubkey.toLowerCase(),
      summary: summary,
      working: working,
      workingNames: [
        for (final pubkey in working)
          labels[pubkey.toLowerCase()] ?? 'An agent',
      ],
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
