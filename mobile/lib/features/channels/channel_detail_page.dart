import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../profile/presence_cache_provider.dart';
import '../profile/profile_provider.dart';
import '../profile/user_cache_provider.dart';
import '../profile/user_profile.dart';
import '../forum/forum_posts_view.dart';
import 'channel.dart';
import 'agent_activity/working_bots_provider.dart';
import 'channel_management_provider.dart';
import 'channel_messages_provider.dart';
import 'channel_typing_provider.dart';
import 'channels_provider.dart';
import 'compose_bar.dart';
import 'date_formatters.dart';
import 'day_divider.dart';
import 'manage_channel_sheet.dart';
import 'members_sheet.dart';
import 'message_actions.dart';
import 'message_content.dart';
import 'read_state/deferred_read_state_update.dart';
import 'read_state/read_state_provider.dart';
import 'read_state/read_state_time.dart';
import 'reaction_row.dart';
import 'send_message_provider.dart';
import '../profile/user_profile_sheet.dart';
import 'small_avatar.dart';
import 'thread_detail_page.dart';
import 'timeline_message.dart';

/// Fetch channel members and preload their profiles into the user cache.
Future<void> _preloadMembers(WidgetRef ref, String channelId) async {
  // Capture references before async gap to avoid using disposed ref.
  final notifier = ref.read(userCacheProvider.notifier);
  try {
    final members = await ref.read(channelMembersProvider(channelId).future);
    final pubkeys = members.map((m) => m.pubkey).toList();
    if (pubkeys.isNotEmpty) {
      notifier.preload(pubkeys);
    }
  } catch (_) {
    // Non-fatal — mentions will just fall back to cache from messages.
  }
}

int? _channelReadTimestamp({
  required Channel channel,
  required AsyncValue<List<NostrEvent>> messagesState,
}) {
  if (channel.isForum) {
    return dateTimeToUnixSeconds(channel.lastMessageAt);
  }

  final events = messagesState.value;
  if (events != null && events.isNotEmpty) {
    var latest = 0;
    for (final event in events) {
      if (event.createdAt > latest) {
        latest = event.createdAt;
      }
    }
    if (latest > 0) {
      return latest;
    }
  }

  return dateTimeToUnixSeconds(channel.lastMessageAt);
}

class ChannelDetailPage extends HookConsumerWidget {
  final Channel channel;

  const ChannelDetailPage({super.key, required this.channel});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final detailsAsync = ref.watch(channelDetailsProvider(channel.id));
    final channelsAsync = ref.watch(channelsProvider);
    final messagesState = ref.watch(channelMessagesProvider(channel.id));
    final readState = ref.watch(readStateProvider);
    final currentPubkey = ref
        .watch(profileProvider)
        .whenData((value) => value?.pubkey)
        .value;
    // Only show channel-level typing (exclude thread-scoped entries and self).
    final typingEntries = ref
        .watch(channelTypingProvider(channel.id))
        .where((e) => e.threadHeadId == null)
        .where(
          (e) =>
              currentPubkey == null ||
              e.pubkey.toLowerCase() != currentPubkey.toLowerCase(),
        )
        .toList();
    final baseChannel =
        channelsAsync
            .whenData(
              (channels) => channels.firstWhere(
                (candidate) => candidate.id == channel.id,
                orElse: () => channel,
              ),
            )
            .value ??
        channel;
    final resolvedChannel =
        detailsAsync.whenData(baseChannel.mergeDetails).value ?? baseChannel;
    final readTimestamp = _channelReadTimestamp(
      channel: resolvedChannel,
      messagesState: messagesState,
    );

    // Preload channel member profiles so @mentions resolve correctly.
    useEffect(() {
      _preloadMembers(ref, channel.id);
      return null;
    }, [channel.id]);

    useEffect(() {
      if (!readState.isReady || readTimestamp == null) {
        return null;
      }
      return deferReadStateUpdate(context, () {
        ref
            .read(readStateProvider.notifier)
            .markContextRead(channel.id, readTimestamp);
      });
    }, [channel.id, readState.isReady, readTimestamp]);

    return FrostedScaffold(
      appBar: FrostedAppBar(
        title: resolvedChannel.isDm
            ? _DmAppBarTitle(
                channel: resolvedChannel,
                currentPubkey: currentPubkey,
              )
            : Row(
                children: [
                  Icon(
                    channelIcon(resolvedChannel),
                    size: 18,
                    color: context.colors.onSurfaceVariant,
                  ),
                  const SizedBox(width: Grid.half),
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        Text(
                          resolvedChannel.displayLabel(
                            currentPubkey: currentPubkey,
                          ),
                          overflow: TextOverflow.ellipsis,
                        ),
                        if (resolvedChannel.isStream)
                          Text(
                            resolvedChannel.description.isNotEmpty
                                ? resolvedChannel.description
                                : '${resolvedChannel.memberCount} member${resolvedChannel.memberCount == 1 ? '' : 's'}',
                            style: context.textTheme.bodySmall?.copyWith(
                              color: context.colors.onSurfaceVariant,
                            ),
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                          ),
                      ],
                    ),
                  ),
                ],
              ),
        actions: [
          _MembersButton(
            channelId: resolvedChannel.id,
            channel: resolvedChannel,
            currentPubkey: currentPubkey,
          ),
          if (!resolvedChannel.isDm)
            IconButton(
              onPressed: () async {
                final shouldClose = await showModalBottomSheet<bool>(
                  context: context,
                  isScrollControlled: true,
                  showDragHandle: true,
                  builder: (_) => ManageChannelSheet(channel: resolvedChannel),
                );
                if (shouldClose == true && context.mounted) {
                  Navigator.of(context).pop();
                }
              },
              tooltip: 'Manage channel',
              icon: const Icon(LucideIcons.ellipsis),
            ),
        ],
      ),
      body: Column(
        children: [
          Expanded(
            child: resolvedChannel.isForum
                ? ForumPostsView(
                    channel: resolvedChannel,
                    currentPubkey: currentPubkey,
                  )
                : messagesState.when(
                    loading: () => Padding(
                      padding: EdgeInsets.only(
                        top: frostedAppBarHeight(context),
                      ),
                      child: const Center(child: CircularProgressIndicator()),
                    ),
                    error: (e, _) => Padding(
                      padding: EdgeInsets.only(
                        top: frostedAppBarHeight(context),
                      ),
                      child: Center(
                        child: Text(
                          'Failed to load messages',
                          style: context.textTheme.bodyMedium?.copyWith(
                            color: context.colors.error,
                          ),
                        ),
                      ),
                    ),
                    data: (events) {
                      final messages = formatTimeline(
                        events,
                        currentPubkey: currentPubkey,
                      );
                      final entries = buildMainTimelineEntries(messages);
                      return _MessageList(
                        entries: entries,
                        allMessages: messages,
                        channelId: channel.id,
                        currentPubkey: currentPubkey,
                        isMember: resolvedChannel.isMember,
                        isArchived: resolvedChannel.isArchived,
                      );
                    },
                  ),
          ),
          _DetailConnectionBanner(
            status: ref.watch(relaySessionProvider).status,
          ),
          if (!resolvedChannel.isForum && typingEntries.isNotEmpty)
            _TypingIndicator(entries: typingEntries),
          if (!resolvedChannel.isForum &&
              resolvedChannel.isMember &&
              !resolvedChannel.isArchived)
            ComposeBar(
              channelId: channel.id,
              channelName: resolvedChannel.isDm ? '' : resolvedChannel.name,
              onSend:
                  (
                    content,
                    mentionPubkeys, {
                    mediaTags = const <List<String>>[],
                  }) => ref
                      .read(sendMessageProvider)
                      .call(
                        channelId: channel.id,
                        content: content,
                        mentionPubkeys: mentionPubkeys,
                        mediaTags: mediaTags,
                      ),
            )
          else if (!resolvedChannel.isDm &&
              (!resolvedChannel.isMember || resolvedChannel.isArchived))
            _ReadOnlyNotice(channel: resolvedChannel),
        ],
      ),
    );
  }
}

class _MessageList extends HookConsumerWidget {
  final List<MainTimelineEntry> entries;
  final List<TimelineMessage> allMessages;
  final String channelId;
  final String? currentPubkey;
  final bool isMember;
  final bool isArchived;

  const _MessageList({
    required this.entries,
    required this.allMessages,
    required this.channelId,
    required this.currentPubkey,
    required this.isMember,
    required this.isArchived,
  });

  static const _fetchOlderThreshold = 200.0;
  static const _latestThreshold = 48.0;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    // Pagination: fetch older messages when scrolling near the top.
    final scrollController = useScrollController();
    final isLoadingOlder = useState(false);
    final isAtLatest = useState(true);
    final latestEntryId = entries.isEmpty ? null : entries.last.message.id;
    final previousLatestEntryId = useRef<String?>(null);

    bool nearLatest() {
      if (!scrollController.hasClients) return true;
      return scrollController.position.pixels <= _latestThreshold;
    }

    void updateLatestState() {
      final next = nearLatest();
      if (isAtLatest.value != next) {
        isAtLatest.value = next;
      }
    }

    Future<void> scrollToLatest() async {
      if (!scrollController.hasClients) return;
      await scrollController.animateTo(
        0,
        duration: const Duration(milliseconds: 220),
        curve: Curves.easeOutCubic,
      );
      if (context.mounted) {
        isAtLatest.value = true;
      }
    }

    useEffect(() {
      void onScroll() {
        updateLatestState();
        if (isLoadingOlder.value) return;
        final notifier = ref.read(channelMessagesProvider(channelId).notifier);
        if (notifier.reachedOldest) return;
        // In a reversed ListView, maxScrollExtent is the oldest messages.
        final pos = scrollController.position;
        if (pos.pixels >= pos.maxScrollExtent - _fetchOlderThreshold) {
          isLoadingOlder.value = true;
          notifier.fetchOlder().whenComplete(
            () => isLoadingOlder.value = false,
          );
        }
      }

      scrollController.addListener(onScroll);
      return () => scrollController.removeListener(onScroll);
    }, [channelId, scrollController]);

    useEffect(() {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!context.mounted) return;
        updateLatestState();
      });
      return null;
    }, [entries.length, scrollController]);

    useEffect(() {
      final previous = previousLatestEntryId.value;
      previousLatestEntryId.value = latestEntryId;
      if (previous == null ||
          latestEntryId == null ||
          previous == latestEntryId ||
          !isAtLatest.value) {
        return null;
      }

      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!context.mounted || !scrollController.hasClients) return;
        scrollController.animateTo(
          0,
          duration: const Duration(milliseconds: 220),
          curve: Curves.easeOutCubic,
        );
      });
      return null;
    }, [latestEntryId, scrollController]);

    if (entries.isEmpty) {
      return Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(
              LucideIcons.messageSquare,
              size: Grid.xl,
              color: context.colors.onSurfaceVariant,
            ),
            const SizedBox(height: Grid.xxs),
            Text(
              'No messages yet',
              style: context.textTheme.bodyLarge?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
            const SizedBox(height: Grid.half),
            Text(
              'Be the first to say something!',
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
          ],
        ),
      );
    }

    // Build channel names map once for all message bubbles.
    final channelsAsync = ref.watch(channelsProvider);
    final channelNamesMap = <String, String>{};
    channelsAsync.whenData((channels) {
      for (final ch in channels) {
        channelNamesMap[ch.name.toLowerCase()] = ch.id;
      }
    });

    return Stack(
      children: [
        ListView.builder(
          key: const ValueKey('channel-message-list'),
          controller: scrollController,
          reverse: true,
          padding: EdgeInsets.only(
            left: Grid.xs,
            right: Grid.xs,
            top: frostedAppBarHeight(context),
            bottom: Grid.xxs,
          ),
          itemCount: entries.length + (isLoadingOlder.value ? 1 : 0),
          itemBuilder: (context, index) {
            // Loading indicator at the top (last index in reversed list).
            if (index >= entries.length) {
              return const Padding(
                padding: EdgeInsets.symmetric(vertical: Grid.xs),
                child: Center(
                  child: SizedBox(
                    width: 20,
                    height: 20,
                    child: CircularProgressIndicator(strokeWidth: 2),
                  ),
                ),
              );
            }

            // Reversed list: index 0 = newest (bottom of screen).
            final chronIdx = entries.length - 1 - index;
            final entry = entries[chronIdx];
            final message = entry.message;

            // Day boundary check — applies to all messages including system.
            final prevEntry = chronIdx > 0 ? entries[chronIdx - 1] : null;
            final prevMessage = prevEntry?.message;
            final showDayDivider =
                prevMessage == null ||
                !isSameDay(prevMessage.createdAt, message.createdAt);

            final showAuthor =
                !message.isSystem &&
                (prevMessage == null ||
                    prevMessage.isSystem ||
                    showDayDivider ||
                    prevMessage.pubkey.toLowerCase() !=
                        message.pubkey.toLowerCase() ||
                    (message.createdAt - prevMessage.createdAt) > 300);

            return Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                if (showDayDivider)
                  DayDivider(label: formatDayHeading(message.createdAt)),
                if (message.isSystem)
                  _SystemMessageRow(
                    message: message,
                    channelId: channelId,
                    currentPubkey: currentPubkey,
                    allMessages: null,
                    isMember: isMember,
                    isArchived: isArchived,
                  )
                else ...[
                  _MessageBubble(
                    message: message,
                    showAuthor: showAuthor,
                    channelNames: channelNamesMap,
                    currentChannelId: channelId,
                    currentPubkey: currentPubkey,
                    allMessages: allMessages,
                    isMember: isMember,
                    isArchived: isArchived,
                  ),
                  if (entry.summary != null)
                    _ThreadSummaryRow(
                      summary: entry.summary!,
                      message: message,
                      allMessages: allMessages,
                      channelId: channelId,
                      currentPubkey: currentPubkey,
                      isMember: isMember,
                      isArchived: isArchived,
                    ),
                ],
              ],
            );
          },
        ),
        if (!isAtLatest.value)
          Positioned(
            left: 0,
            right: 0,
            bottom: Grid.xs,
            child: Center(
              child: FilledButton.icon(
                key: const ValueKey('channel-jump-to-latest'),
                onPressed: scrollToLatest,
                style: FilledButton.styleFrom(
                  backgroundColor: context.colors.primaryContainer,
                  foregroundColor: context.colors.onPrimaryContainer,
                  padding: const EdgeInsets.symmetric(
                    horizontal: Grid.xs,
                    vertical: Grid.xxs,
                  ),
                ),
                icon: const Icon(LucideIcons.arrowDown, size: 16),
                label: const Text('Latest'),
              ),
            ),
          ),
      ],
    );
  }
}

class _SystemMessageRow extends ConsumerWidget {
  final TimelineMessage message;
  final String channelId;
  final String? currentPubkey;
  final List<TimelineMessage>? allMessages;
  final bool isMember;
  final bool isArchived;

  const _SystemMessageRow({
    required this.message,
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

    String resolveLabel(String? pubkey) {
      if (pubkey == null) return 'Someone';
      final profile =
          userCache[pubkey.toLowerCase()] ??
          ref.read(userCacheProvider.notifier).get(pubkey.toLowerCase());
      return profile?.label ?? shortPubkey(pubkey);
    }

    final description = systemEvent.describe(resolveLabel);

    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onLongPress: () => showMessageActions(
        context: context,
        ref: ref,
        message: message,
        channelId: channelId,
        isOwnMessage: false,
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
            Row(
              children: [
                _systemEventAvatar(context, systemEvent, userCache),
                const SizedBox(width: Grid.xxs),
                Expanded(
                  child: Text(
                    description,
                    style: context.textTheme.bodySmall?.copyWith(
                      color: context.colors.onSurfaceVariant,
                    ),
                  ),
                ),
                Text(
                  formatMessageTime(message.createdAt),
                  style: context.textTheme.labelSmall?.copyWith(
                    color: context.colors.onSurfaceVariant,
                  ),
                ),
              ],
            ),
            if (message.reactions.isNotEmpty)
              Padding(
                padding: const EdgeInsets.only(left: 28),
                child: ReactionRow(
                  reactions: message.reactions,
                  onToggle: (emoji) => toggleReaction(ref, message, emoji),
                ),
              ),
          ],
        ),
      ),
    );
  }
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
          left: 36,
          top: Grid.half,
          bottom: Grid.half,
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            // Stacked participant avatars.
            SizedBox(
              width: 20.0 + (summary.participantPubkeys.length - 1) * 12.0,
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

class _MessageBubble extends ConsumerWidget {
  final TimelineMessage message;
  final bool showAuthor;
  final Map<String, String> channelNames;
  final String currentChannelId;
  final String? currentPubkey;
  final List<TimelineMessage>? allMessages;
  final bool isMember;
  final bool isArchived;

  const _MessageBubble({
    required this.message,
    required this.showAuthor,
    required this.channelNames,
    required this.currentChannelId,
    required this.currentPubkey,
    this.allMessages,
    this.isMember = false,
    this.isArchived = false,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    // Watch only this user's profile to avoid rebuilding on unrelated cache changes.
    final pk = message.pubkey.toLowerCase();
    final profile =
        ref.watch(userCacheProvider.select((cache) => cache[pk])) ??
        ref.read(userCacheProvider.notifier).get(pk);
    final displayName = profile?.label ?? shortPubkey(message.pubkey);

    // Build mention names map from event p-tags.
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
        channelId: currentChannelId,
        isOwnMessage:
            currentPubkey?.toLowerCase() == message.pubkey.toLowerCase(),
        allMessages: allMessages,
        currentPubkey: currentPubkey,
        isMember: isMember,
        isArchived: isArchived,
      ),
      child: Padding(
        padding: EdgeInsets.only(top: showAuthor ? Grid.xs : Grid.half),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            if (showAuthor)
              GestureDetector(
                onTap: () => showUserProfileSheet(context, message.pubkey),
                child: _UserAvatar(profile: profile, pubkey: message.pubkey),
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
                    onChannelTap: (channelId) {
                      if (channelId == currentChannelId) return;
                      final channelsAsync = ref.read(channelsProvider);
                      final channels = channelsAsync.hasValue
                          ? channelsAsync.value
                          : null;
                      Channel? targetChannel;
                      for (final channel in channels ?? const <Channel>[]) {
                        if (channel.id == channelId) {
                          targetChannel = channel;
                          break;
                        }
                      }
                      if (targetChannel == null) return;
                      Navigator.of(context).push(
                        MaterialPageRoute<void>(
                          builder: (_) =>
                              ChannelDetailPage(channel: targetChannel!),
                        ),
                      );
                    },
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

class _UserAvatar extends StatelessWidget {
  final UserProfile? profile;
  final String pubkey;

  const _UserAvatar({required this.profile, required this.pubkey});

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

IconData channelIcon(Channel channel) {
  if (channel.isDm) return LucideIcons.messagesSquare;
  if (channel.isPrivate) return LucideIcons.lock;
  if (channel.isForum) return LucideIcons.messageSquareText;
  return LucideIcons.hash;
}

class _ReadOnlyNotice extends StatelessWidget {
  final Channel channel;

  const _ReadOnlyNotice({required this.channel});

  @override
  Widget build(BuildContext context) {
    return Container(
      width: double.infinity,
      padding: EdgeInsets.only(
        left: Grid.xs,
        right: Grid.xs,
        top: Grid.xxs,
        bottom: MediaQuery.viewPaddingOf(context).bottom + Grid.xxs,
      ),
      decoration: BoxDecoration(
        border: Border(top: BorderSide(color: context.colors.outlineVariant)),
        color: context.colors.surface,
      ),
      child: Text(
        channel.isArchived
            ? 'This ${channel.isForum ? 'forum' : 'channel'} is archived and read-only on mobile.'
            : 'Join this ${channel.isForum ? 'forum' : 'channel'} from Manage to participate.',
        style: context.textTheme.bodySmall?.copyWith(
          color: context.colors.onSurfaceVariant,
        ),
        textAlign: TextAlign.center,
      ),
    );
  }
}

class _DetailConnectionBanner extends StatelessWidget {
  final SessionStatus status;

  const _DetailConnectionBanner({required this.status});

  @override
  Widget build(BuildContext context) {
    if (status == SessionStatus.connected ||
        status == SessionStatus.disconnected) {
      return const SizedBox.shrink();
    }

    return Container(
      width: double.infinity,
      padding: const EdgeInsets.symmetric(
        horizontal: Grid.xs,
        vertical: Grid.quarter + 2,
      ),
      color: context.colors.surfaceContainerHighest,
      child: Row(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          SizedBox(
            width: 12,
            height: 12,
            child: CircularProgressIndicator(
              strokeWidth: 2,
              color: context.colors.onSurfaceVariant,
            ),
          ),
          const SizedBox(width: Grid.xxs),
          Text(
            'Reconnecting…',
            style: context.textTheme.labelSmall?.copyWith(
              color: context.colors.onSurfaceVariant,
            ),
          ),
        ],
      ),
    );
  }
}

class _TypingIndicator extends ConsumerWidget {
  final List<TypingEntry> entries;

  const _TypingIndicator({required this.entries});

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
      1 => '${names[0]} is typing…',
      2 => '${names[0]} and ${names[1]} are typing…',
      _ => '${names[0]} and ${names.length - 1} others are typing…',
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

class _MembersButton extends ConsumerWidget {
  final String channelId;
  final Channel channel;
  final String? currentPubkey;

  const _MembersButton({
    required this.channelId,
    required this.channel,
    required this.currentPubkey,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final hasWorkingBot = ref
        .watch(workingBotPubkeysProvider(channelId))
        .isNotEmpty;

    return IconButton(
      onPressed: () {
        showModalBottomSheet<void>(
          context: context,
          isScrollControlled: true,
          showDragHandle: true,
          builder: (_) =>
              MembersSheet(channel: channel, currentPubkey: currentPubkey),
        );
      },
      tooltip: 'View members',
      icon: Stack(
        clipBehavior: Clip.none,
        children: [
          const Icon(LucideIcons.users),
          if (hasWorkingBot)
            Positioned(
              top: -2,
              right: -2,
              child: Container(
                width: 8,
                height: 8,
                decoration: BoxDecoration(
                  color: context.appColors.success,
                  shape: BoxShape.circle,
                  border: Border.all(color: context.colors.surface, width: 1.5),
                ),
              ),
            ),
        ],
      ),
    );
  }
}

class _DmAppBarTitle extends ConsumerWidget {
  final Channel channel;
  final String? currentPubkey;

  const _DmAppBarTitle({required this.channel, required this.currentPubkey});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final profiles = ref.watch(userCacheProvider);
    final presenceMap = ref.watch(presenceCacheProvider);
    final normalizedCurrent = currentPubkey?.toLowerCase();

    String? otherPubkey;
    for (final pk in channel.participantPubkeys) {
      if (pk.toLowerCase() != normalizedCurrent) {
        otherPubkey = pk.toLowerCase();
        break;
      }
    }

    final profile = otherPubkey != null ? profiles[otherPubkey] : null;

    if (otherPubkey != null) {
      if (profile == null) {
        ref.read(userCacheProvider.notifier).preload([otherPubkey]);
      }
      ref.read(presenceCacheProvider.notifier).track([otherPubkey]);
    }

    final avatarUrl = profile?.avatarUrl;
    final initial =
        profile?.initial ??
        (channel.participants.isNotEmpty
            ? channel.participants.first[0].toUpperCase()
            : '?');
    final presence = otherPubkey != null
        ? (presenceMap[otherPubkey] ?? 'offline')
        : 'offline';
    final presenceLabel = switch (presence) {
      'online' => 'Online',
      'away' => 'Away',
      _ => 'Offline',
    };

    return Row(
      children: [
        SizedBox(
          width: 30,
          height: 30,
          child: Stack(
            clipBehavior: Clip.none,
            children: [
              CircleAvatar(
                radius: 14,
                backgroundColor: context.colors.primaryContainer,
                backgroundImage: avatarUrl != null
                    ? NetworkImage(avatarUrl)
                    : null,
                child: avatarUrl == null
                    ? Text(
                        initial,
                        style: context.textTheme.labelSmall?.copyWith(
                          color: context.colors.onPrimaryContainer,
                          fontWeight: FontWeight.w600,
                        ),
                      )
                    : null,
              ),
              Positioned(
                right: -1,
                bottom: -1,
                child: Container(
                  width: 10,
                  height: 10,
                  decoration: BoxDecoration(
                    color: switch (presence) {
                      'online' => context.appColors.success,
                      'away' => context.appColors.warning,
                      _ => context.colors.outline,
                    },
                    shape: BoxShape.circle,
                    border: Border.all(
                      color: context.colors.surface,
                      width: 1.5,
                    ),
                  ),
                ),
              ),
            ],
          ),
        ),
        const SizedBox(width: Grid.xxs),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              Text(
                channel.displayLabel(currentPubkey: currentPubkey),
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: context.textTheme.titleSmall,
              ),
              Text(
                presenceLabel,
                style: context.textTheme.bodySmall?.copyWith(
                  color: context.colors.onSurfaceVariant,
                ),
              ),
            ],
          ),
        ),
      ],
    );
  }
}
