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
import 'channel_link_navigation.dart';
import 'agent_activity/working_bots_provider.dart';
import 'channel_management_provider.dart';
import 'channel_messages_provider.dart';
import 'channel_typing_provider.dart';
import 'channels_provider.dart';
import 'compose_bar.dart';
import 'date_formatters.dart';
import 'day_divider.dart';
import 'dm_channel_labels.dart';
import 'ephemeral_channel_display.dart';
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

part 'channel_detail_page/message_list.dart';
part 'channel_detail_page/system_rows.dart';
part 'channel_detail_page/message_bubble.dart';
part 'channel_detail_page/banners.dart';
part 'channel_detail_page/app_bar.dart';

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
      if (event.threadReference.parentId != null) continue;
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
        ref
            .read(channelsProvider.notifier)
            .clearObservedUnreadCoveredByRead(channel.id, readTimestamp);
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
                        Row(
                          mainAxisSize: MainAxisSize.min,
                          children: [
                            Flexible(
                              child: Text(
                                resolveDmChannelDisplayLabel(
                                  resolvedChannel,
                                  currentPubkey: currentPubkey,
                                ),
                                overflow: TextOverflow.ellipsis,
                              ),
                            ),
                            if (resolvedChannel.isEphemeral) ...[
                              const SizedBox(width: Grid.quarter),
                              _HeaderEphemeralBadge(channel: resolvedChannel),
                            ],
                          ],
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
                  constraints: BoxConstraints(
                    maxWidth: 640,
                    maxHeight: MediaQuery.sizeOf(context).height * 0.9,
                  ),
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
                      final summaries = ref
                          .read(channelMessagesProvider(channel.id).notifier)
                          .threadSummaries;
                      final entries = buildMainTimelineEntries(
                        messages,
                        relaySummaries: summaries,
                      );
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
