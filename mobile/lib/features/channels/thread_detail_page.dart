import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../profile/user_cache_provider.dart';
import '../profile/user_profile.dart';
import 'channel_messages_provider.dart';
import 'channel_typing_provider.dart';
import 'channels_provider.dart';
import 'compose_bar.dart';
import 'date_formatters.dart';
import '../profile/user_profile_sheet.dart';
import 'message_actions.dart';
import 'message_content.dart';
import 'reaction_row.dart';
import 'send_message_provider.dart';
import 'small_avatar.dart';
import 'timeline_message.dart';

/// Full-screen thread detail page.
///
/// Shows the thread head message, direct replies, typing indicators scoped to
/// the thread, and a compose bar for replying.
class ThreadDetailPage extends HookConsumerWidget {
  final TimelineMessage threadHead;
  final List<TimelineMessage> allMessages;
  final String channelId;
  final String? currentPubkey;
  final bool isMember;
  final bool isArchived;

  const ThreadDetailPage({
    super.key,
    required this.threadHead,
    required this.allMessages,
    required this.channelId,
    required this.currentPubkey,
    required this.isMember,
    required this.isArchived,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    // Re-derive replies from live message state so new replies appear.
    final messagesState = ref.watch(channelMessagesProvider(channelId));
    final liveMessages = messagesState.whenData((events) {
      return formatTimeline(events, currentPubkey: currentPubkey);
    });

    final allMsgs = liveMessages.value ?? allMessages;

    // Index all messages by parentId so we can find direct children of any
    // message and compute thread summaries for nested threads.
    final childrenByParent = <String, List<TimelineMessage>>{};
    for (final msg in allMsgs) {
      final pid = msg.parentId;
      if (pid == null) continue;
      childrenByParent.putIfAbsent(pid, () => []).add(msg);
    }

    final replies = childrenByParent[threadHead.id] ?? const [];

    // Thread-scoped typing indicators (exclude self).
    final allTyping = ref.watch(channelTypingProvider(channelId));
    final threadTyping = allTyping
        .where((e) => e.threadHeadId == threadHead.id)
        .where(
          (e) =>
              currentPubkey == null ||
              e.pubkey.toLowerCase() != currentPubkey?.toLowerCase(),
        )
        .toList();

    // Resolve thread head from live data (reactions/edits may have changed).
    final liveHead =
        allMsgs.where((m) => m.id == threadHead.id).firstOrNull ?? threadHead;

    // The root of the entire thread chain. If the current thread head is
    // itself a root message its rootId is null, so fall back to its own id.
    final effectiveRootId = threadHead.rootId ?? threadHead.id;

    // Channel names for message content rendering.
    final channelsAsync = ref.watch(channelsProvider);
    final channelNamesMap = <String, String>{};
    channelsAsync.whenData((channels) {
      for (final ch in channels) {
        channelNamesMap[ch.name.toLowerCase()] = ch.id;
      }
    });

    return FrostedScaffold(
      appBar: const FrostedAppBar(title: Text('Thread')),
      body: Column(
        children: [
          Expanded(
            child: ListView.builder(
              padding: EdgeInsets.only(
                left: Grid.xs,
                right: Grid.xs,
                top: frostedAppBarHeight(context),
                bottom: Grid.xxs,
              ),
              itemCount: replies.length + 1, // +1 for thread head
              itemBuilder: (context, index) {
                if (index == 0) {
                  // Thread head.
                  return Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      _ThreadMessage(
                        message: liveHead,
                        channelNames: channelNamesMap,
                        channelId: channelId,
                        currentPubkey: currentPubkey,
                        showAuthor: true,
                        allMessages: allMsgs,
                        isMember: isMember,
                        isArchived: isArchived,
                      ),
                      Padding(
                        padding: const EdgeInsets.symmetric(vertical: Grid.xxs),
                        child: Row(
                          children: [
                            Text(
                              '${replies.length} ${replies.length == 1 ? 'reply' : 'replies'}',
                              style: context.textTheme.labelMedium?.copyWith(
                                color: context.colors.onSurfaceVariant,
                                fontWeight: FontWeight.w600,
                              ),
                            ),
                            const SizedBox(width: Grid.xxs),
                            Expanded(
                              child: Divider(
                                color: context.colors.outlineVariant,
                              ),
                            ),
                          ],
                        ),
                      ),
                    ],
                  );
                }

                final reply = replies[index - 1];
                final prevReply = index > 1 ? replies[index - 2] : null;
                final showAuthor =
                    prevReply == null ||
                    prevReply.pubkey.toLowerCase() !=
                        reply.pubkey.toLowerCase() ||
                    (reply.createdAt - prevReply.createdAt) > 300;

                // Check if this reply itself has children (nested thread).
                final nestedChildren = childrenByParent[reply.id];
                final nestedSummary =
                    nestedChildren != null && nestedChildren.isNotEmpty
                    ? _buildNestedSummary(reply.id, nestedChildren)
                    : null;

                return Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    _ThreadMessage(
                      message: reply,
                      channelNames: channelNamesMap,
                      channelId: channelId,
                      currentPubkey: currentPubkey,
                      showAuthor: showAuthor,
                      allMessages: allMsgs,
                      isMember: isMember,
                      isArchived: isArchived,
                    ),
                    if (nestedSummary != null)
                      _NestedThreadSummaryRow(
                        summary: nestedSummary,
                        replyMessage: reply,
                        allMessages: allMsgs,
                        channelId: channelId,
                        currentPubkey: currentPubkey,
                        isMember: isMember,
                        isArchived: isArchived,
                      ),
                  ],
                );
              },
            ),
          ),
          if (threadTyping.isNotEmpty)
            _ThreadTypingIndicator(entries: threadTyping),
          if (isMember && !isArchived)
            ComposeBar(
              channelId: channelId,
              hintText: 'Reply in thread\u2026',
              threadHeadId: threadHead.id,
              rootId: effectiveRootId,
              onSend:
                  (
                    content,
                    mentionPubkeys, {
                    mediaTags = const <List<String>>[],
                  }) => ref
                      .read(sendMessageProvider)
                      .call(
                        channelId: channelId,
                        content: content,
                        mentionPubkeys: mentionPubkeys,
                        parentEventId: threadHead.id,
                        rootEventId: effectiveRootId,
                        mediaTags: mediaTags,
                      ),
            ),
        ],
      ),
    );
  }
}

/// Build a lightweight summary for a nested thread (reply that has its own
/// replies). Same logic as the top-level [ThreadSummary] but kept local to
/// avoid coupling.
ThreadSummary _buildNestedSummary(
  String messageId,
  List<TimelineMessage> children,
) {
  final seen = <String>{};
  final participants = <String>[];
  for (var i = children.length - 1; i >= 0 && participants.length < 3; i--) {
    final pk = children[i].pubkey.toLowerCase();
    if (seen.add(pk)) participants.add(pk);
  }
  return ThreadSummary(
    threadHeadId: messageId,
    replyCount: children.length,
    participantPubkeys: participants.reversed.toList(),
  );
}

/// Tappable summary row shown below a reply that itself has replies.
/// Pushes a new [ThreadDetailPage] for the nested thread.
class _NestedThreadSummaryRow extends ConsumerWidget {
  final ThreadSummary summary;
  final TimelineMessage replyMessage;
  final List<TimelineMessage> allMessages;
  final String channelId;
  final String? currentPubkey;
  final bool isMember;
  final bool isArchived;

  const _NestedThreadSummaryRow({
    required this.summary,
    required this.replyMessage,
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
              threadHead: replyMessage,
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
          left: 36,
          top: Grid.half,
          bottom: Grid.half,
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            // Stacked participant avatars.
            SizedBox(
              width:
                  20.0 +
                  (summary.participantPubkeys.length - 1).clamp(0, 2) * 12.0,
              height: 20,
              child: Stack(
                children: [
                  for (var i = 0; i < summary.participantPubkeys.length; i++)
                    Positioned(
                      left: i * 12.0,
                      child: SmallAvatar(
                        pubkey: summary.participantPubkeys[i],
                        userCache: userCache,
                      ),
                    ),
                ],
              ),
            ),
            const SizedBox(width: Grid.xxs),
            Text(
              '${summary.replyCount} ${summary.replyCount == 1 ? 'reply' : 'replies'}',
              style: context.textTheme.labelMedium?.copyWith(
                color: context.colors.primary,
                fontWeight: FontWeight.w600,
              ),
            ),
            const SizedBox(width: Grid.half),
            Icon(
              LucideIcons.chevronRight,
              size: 14,
              color: context.colors.primary,
            ),
          ],
        ),
      ),
    );
  }
}

class _ThreadMessage extends ConsumerWidget {
  final TimelineMessage message;
  final Map<String, String> channelNames;
  final String channelId;
  final String? currentPubkey;
  final bool showAuthor;
  final List<TimelineMessage>? allMessages;
  final bool isMember;
  final bool isArchived;

  const _ThreadMessage({
    required this.message,
    required this.channelNames,
    required this.channelId,
    required this.currentPubkey,
    required this.showAuthor,
    this.allMessages,
    this.isMember = false,
    this.isArchived = false,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final pk = message.pubkey.toLowerCase();
    final profile =
        ref.watch(userCacheProvider.select((cache) => cache[pk])) ??
        ref.read(userCacheProvider.notifier).get(pk);
    final displayName = profile?.label ?? shortPubkey(message.pubkey);

    final userCache = ref.watch(userCacheProvider);
    final mentionNames = <String, String>{};
    for (final mpk in message.mentionPubkeys) {
      final p = userCache[mpk.toLowerCase()];
      if (p?.displayName != null) {
        mentionNames[mpk.toLowerCase()] = p!.displayName!;
      }
    }

    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onLongPress: () => showMessageActions(
        context: context,
        ref: ref,
        message: message,
        channelId: channelId,
        isOwnMessage:
            currentPubkey?.toLowerCase() == message.pubkey.toLowerCase(),
        allMessages: allMessages,
        currentPubkey: currentPubkey,
        isMember: isMember,
        isArchived: isArchived,
      ),
      child: Padding(
        padding: EdgeInsets.only(top: showAuthor ? Grid.xs : Grid.quarter),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            if (showAuthor)
              GestureDetector(
                onTap: () => showUserProfileSheet(context, message.pubkey),
                child: _Avatar(profile: profile, pubkey: message.pubkey),
              )
            else
              const SizedBox(width: 28),
            const SizedBox(width: Grid.xxs),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  if (showAuthor)
                    Padding(
                      padding: const EdgeInsets.only(bottom: Grid.quarter),
                      child: Row(
                        children: [
                          GestureDetector(
                            onTap: () =>
                                showUserProfileSheet(context, message.pubkey),
                            child: Text(
                              displayName,
                              style: context.textTheme.labelMedium?.copyWith(
                                fontWeight: FontWeight.w600,
                                color: context.colors.onSurface,
                              ),
                            ),
                          ),
                          const SizedBox(width: Grid.xxs),
                          Text(
                            formatMessageTime(message.createdAt),
                            style: context.textTheme.labelSmall?.copyWith(
                              color: context.colors.onSurfaceVariant,
                            ),
                          ),
                          if (message.edited) ...[
                            const SizedBox(width: Grid.half),
                            Text(
                              '(edited)',
                              style: context.textTheme.labelSmall?.copyWith(
                                color: context.colors.onSurfaceVariant,
                                fontStyle: FontStyle.italic,
                              ),
                            ),
                          ],
                        ],
                      ),
                    ),
                  MessageContent(
                    content: message.content,
                    mentionNames: mentionNames,
                    channelNames: channelNames,
                    tags: message.tags,
                    onChannelTap: (_) {},
                  ),
                  if (message.reactions.isNotEmpty)
                    ReactionRow(
                      reactions: message.reactions,
                      onToggle: (emoji) => toggleReaction(ref, message, emoji),
                    ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _ThreadTypingIndicator extends ConsumerWidget {
  final List<TypingEntry> entries;

  const _ThreadTypingIndicator({required this.entries});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final userCache = ref.watch(userCacheProvider);
    final names = entries.map((e) {
      final profile =
          userCache[e.pubkey.toLowerCase()] ??
          ref.read(userCacheProvider.notifier).get(e.pubkey.toLowerCase());
      return profile?.label ?? shortPubkey(e.pubkey);
    }).toList();
    final text = switch (names.length) {
      1 => '${names[0]} is typing...',
      2 => '${names[0]} and ${names[1]} are typing...',
      _ => '${names[0]} and ${names.length - 1} others are typing...',
    };

    final visibleEntries = entries.take(3).toList();
    final avatarCount = visibleEntries.length;

    return Container(
      width: double.infinity,
      padding: const EdgeInsets.symmetric(
        horizontal: Grid.xs,
        vertical: Grid.quarter + 2,
      ),
      child: Row(
        children: [
          SizedBox(
            width: 20.0 + (avatarCount - 1) * 12.0,
            height: 20,
            child: Stack(
              children: [
                for (var i = 0; i < avatarCount; i++)
                  Positioned(
                    left: i * 12.0,
                    child: SmallAvatar(
                      pubkey: visibleEntries[i].pubkey,
                      userCache: userCache,
                    ),
                  ),
              ],
            ),
          ),
          const SizedBox(width: Grid.xxs),
          Flexible(
            child: Text(
              text,
              style: context.textTheme.labelSmall?.copyWith(
                color: context.colors.outline,
                fontStyle: FontStyle.italic,
              ),
              overflow: TextOverflow.ellipsis,
            ),
          ),
        ],
      ),
    );
  }
}

class _Avatar extends StatelessWidget {
  final UserProfile? profile;
  final String pubkey;

  const _Avatar({required this.profile, required this.pubkey});

  @override
  Widget build(BuildContext context) {
    final initial =
        profile?.initial ?? (pubkey.isNotEmpty ? pubkey[0].toUpperCase() : '?');
    final avatarUrl = profile?.avatarUrl;

    return CircleAvatar(
      radius: 14,
      backgroundColor: context.colors.primaryContainer,
      backgroundImage: avatarUrl != null ? NetworkImage(avatarUrl) : null,
      child: avatarUrl == null
          ? Text(
              initial,
              style: context.textTheme.labelSmall?.copyWith(
                color: context.colors.onPrimaryContainer,
                fontWeight: FontWeight.w600,
              ),
            )
          : null,
    );
  }
}
