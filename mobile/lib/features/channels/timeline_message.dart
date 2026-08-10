import 'dart:convert';

import 'package:flutter/foundation.dart';

import '../../shared/relay/relay.dart';
import '../../shared/custom_emoji/custom_emoji.dart';
import 'channel_window.dart';

enum SystemEventType {
  memberJoined,
  memberLeft,
  memberRemoved,
  topicChanged,
  purposeChanged,
  channelCreated,
  channelArchived,
  channelUnarchived,
  huddleStarted,
  huddleEnded,
}

@immutable
class SystemEvent {
  final SystemEventType type;
  final String? actorPubkey;
  final String? targetPubkey;
  final String? topic;
  final String? purpose;

  const SystemEvent({
    required this.type,
    this.actorPubkey,
    this.targetPubkey,
    this.topic,
    this.purpose,
  });

  /// Parse a system event from the JSON content of a kind-40099 event.
  /// Returns null if the payload is unrecognised.
  static SystemEvent? fromContent(String content) {
    final Map<dynamic, dynamic> json;
    try {
      final decoded = jsonDecode(content);
      if (decoded is! Map) {
        return null;
      }
      json = decoded;
    } catch (_) {
      return null;
    }

    final type = switch (_readString(json, 'type')) {
      'member_joined' => SystemEventType.memberJoined,
      'member_left' => SystemEventType.memberLeft,
      'member_removed' => SystemEventType.memberRemoved,
      'topic_changed' => SystemEventType.topicChanged,
      'purpose_changed' => SystemEventType.purposeChanged,
      'channel_created' => SystemEventType.channelCreated,
      'channel_archived' => SystemEventType.channelArchived,
      'channel_unarchived' => SystemEventType.channelUnarchived,
      _ => null,
    };

    if (type == null) return null;

    return SystemEvent(
      type: type,
      actorPubkey: _readString(json, 'actor'),
      targetPubkey: _readString(json, 'target'),
      topic: _readString(json, 'topic'),
      purpose: _readString(json, 'purpose'),
    );
  }

  static SystemEvent? fromHuddleEvent(NostrEvent event) {
    final type = switch (event.kind) {
      EventKind.huddleStarted => SystemEventType.huddleStarted,
      EventKind.huddleEnded => SystemEventType.huddleEnded,
      _ => null,
    };
    if (type == null) return null;

    return SystemEvent(type: type, actorPubkey: event.pubkey);
  }

  /// Human-readable description. [resolveLabel] maps a pubkey to a display
  /// name — the caller provides it so this class stays free of provider deps.
  String describe(String Function(String? pubkey) resolveLabel) {
    final actor = resolveLabel(actorPubkey);

    return switch (type) {
      SystemEventType.memberJoined => () {
        if (actorPubkey != null && actorPubkey == targetPubkey) {
          return '$actor joined the channel';
        }
        final target = resolveLabel(targetPubkey);
        return '$target was added by $actor';
      }(),
      SystemEventType.memberLeft => '$actor left the channel',
      SystemEventType.memberRemoved => () {
        final target = resolveLabel(targetPubkey);
        return '$actor removed $target from the channel';
      }(),
      SystemEventType.topicChanged =>
        '$actor ${_describeTextFieldChange('topic', topic)}',
      SystemEventType.purposeChanged =>
        '$actor ${_describeTextFieldChange('purpose', purpose)}',
      SystemEventType.channelCreated => '$actor created this channel',
      SystemEventType.channelArchived => '$actor archived this channel',
      SystemEventType.channelUnarchived => '$actor unarchived this channel',
      SystemEventType.huddleStarted => '$actor started a huddle',
      SystemEventType.huddleEnded => '$actor ended the huddle',
    };
  }
}

/// Caption fragment for a channel topic or purpose change, e.g.
/// `changed the topic to "Release planning"` or `cleared the topic`.
///
/// A blank value means the field was cleared: the relay reports a clear as a
/// `topic_changed` / `purpose_changed` event carrying an empty string, not as a
/// separate event type. Without this branch the timeline renders
/// `changed the topic to ""`, which reads as if the topic were set to two quote
/// marks. Whitespace-only values are treated as cleared for the same reason.
///
/// Mirrors `describeChannelTextFieldChange` in
/// `desktop/src/features/messages/lib/systemEventCopy.ts`.
String _describeTextFieldChange(String field, String? value) {
  final trimmed = value?.trim();
  if (trimmed == null || trimmed.isEmpty) {
    return 'cleared the $field';
  }
  return 'changed the $field to "$trimmed"';
}

@immutable
class TimelineReaction {
  final String emoji;
  final int count;
  final bool reactedByCurrentUser;
  final List<String> userPubkeys;
  final String? emojiUrl;

  /// The event ID of the current user's reaction, for deletion.
  final String? currentUserReactionId;

  const TimelineReaction({
    required this.emoji,
    required this.count,
    required this.reactedByCurrentUser,
    required this.userPubkeys,
    this.emojiUrl,
    this.currentUserReactionId,
  });
}

@immutable
class TimelineMessage {
  final String id;
  final String pubkey;
  final int createdAt;
  final String content;
  final List<List<String>> tags;
  final bool isSystem;
  final bool edited;
  final SystemEvent? systemEvent;

  /// Pubkeys mentioned in this message (from p-tags).
  final List<String> mentionPubkeys;

  /// Aggregated reactions on this message.
  final List<TimelineReaction> reactions;

  /// Direct parent event ID (null for top-level messages).
  final String? parentId;

  /// Root event ID of the thread (null for top-level messages).
  final String? rootId;

  const TimelineMessage({
    required this.id,
    required this.pubkey,
    required this.createdAt,
    required this.content,
    this.tags = const [],
    this.isSystem = false,
    this.edited = false,
    this.systemEvent,
    this.mentionPubkeys = const [],
    this.reactions = const [],
    this.parentId,
    this.rootId,
  });

  /// Attachment messages stay visually distinct from surrounding messages,
  /// even when several are sent by the same author in quick succession.
  bool get hasAttachments =>
      tags.any((tag) => tag.isNotEmpty && tag.first == 'imeta');
}

@immutable
class ThreadSummary {
  final String threadHeadId;
  final int replyCount;

  /// Up to 3 most recent unique participant pubkeys.
  final List<String> participantPubkeys;
  final int? lastReplyAt;

  const ThreadSummary({
    required this.threadHeadId,
    required this.replyCount,
    required this.participantPubkeys,
    this.lastReplyAt,
  });
}

/// A main-timeline entry: a root message with an optional thread summary.
@immutable
class MainTimelineEntry {
  final TimelineMessage message;
  final ThreadSummary? summary;

  const MainTimelineEntry({required this.message, this.summary});
}

const _membershipGroupWindowSeconds = 5 * 60;

@immutable
class _MembershipChange {
  final String? actor;
  final bool isSelfJoin;

  const _MembershipChange({required this.actor, required this.isSelfJoin});
}

_MembershipChange? _membershipChange(MainTimelineEntry entry) {
  final event = entry.message.systemEvent;
  if (!entry.message.isSystem || event?.type != SystemEventType.memberJoined) {
    return null;
  }

  final actor = event?.actorPubkey?.trim().toLowerCase();
  final target = event?.targetPubkey?.trim().toLowerCase();
  if (actor == null || actor.isEmpty || target == null || target.isEmpty) {
    return null;
  }

  final isSelfJoin = actor == target;
  return _MembershipChange(
    actor: isSelfJoin ? null : actor,
    isSelfJoin: isSelfJoin,
  );
}

bool _membershipChangesCanGroup(
  _MembershipChange first,
  _MembershipChange second,
) {
  return first.isSelfJoin == second.isSelfJoin &&
      (first.isSelfJoin || first.actor == second.actor);
}

bool _isSameLocalDay(int firstTimestamp, int secondTimestamp) {
  final first = DateTime.fromMillisecondsSinceEpoch(firstTimestamp * 1000);
  final second = DateTime.fromMillisecondsSinceEpoch(secondTimestamp * 1000);
  return first.year == second.year &&
      first.month == second.month &&
      first.day == second.day;
}

/// Groups consecutive membership arrivals using the same display rule as
/// desktop: matching additions (or self-joins) within a fixed five-minute
/// window become one render item. Other events and local day boundaries break
/// the group.
///
/// Each inner list is one renderable timeline item. Non-grouped entries are
/// returned as single-item lists.
List<List<MainTimelineEntry>> groupMembershipTimelineEntries(
  List<MainTimelineEntry> entries,
) {
  final groupsByStart = <int, int>{};

  for (var end = entries.length - 1; end >= 0;) {
    final newestEntry = entries[end];
    final newestChange = _membershipChange(newestEntry);
    if (newestChange == null) {
      end -= 1;
      continue;
    }

    var start = end;
    while (start > 0) {
      final candidate = entries[start - 1];
      final candidateChange = _membershipChange(candidate);
      if (candidateChange == null ||
          !_membershipChangesCanGroup(candidateChange, newestChange) ||
          !_isSameLocalDay(
            candidate.message.createdAt,
            newestEntry.message.createdAt,
          ) ||
          newestEntry.message.createdAt < candidate.message.createdAt ||
          newestEntry.message.createdAt - candidate.message.createdAt >
              _membershipGroupWindowSeconds) {
        break;
      }
      start -= 1;
    }

    if (start < end) groupsByStart[start] = end;
    end = start - 1;
  }

  final result = <List<MainTimelineEntry>>[];
  for (var index = 0; index < entries.length;) {
    final groupEnd = groupsByStart[index];
    if (groupEnd == null) {
      result.add([entries[index]]);
      index += 1;
      continue;
    }
    result.add(entries.sublist(index, groupEnd + 1));
    index = groupEnd + 1;
  }
  return result;
}

/// Process a chronologically-sorted list of [NostrEvent]s into a list of
/// [TimelineMessage]s, applying deletions, edits, reactions, and system event
/// parsing.
///
/// Mirrors the desktop's `formatTimelineMessages` logic.
/// [currentPubkey] is used to determine if the current user has reacted.
List<TimelineMessage> formatTimeline(
  List<NostrEvent> events, {
  String? currentPubkey,
}) {
  // 1. Collect deletion targets. Both kind:5 (NIP-09) and kind:9005
  // (Buzz-native) are deletion markers; mirror desktop's behavior.
  final deletedIds = <String>{};
  for (final event in events) {
    if (event.kind != EventKind.deletion &&
        event.kind != EventKind.nip29DeleteEvent) {
      continue;
    }
    for (final tag in event.tags) {
      if (tag.length >= 2 && tag[0] == 'e') {
        deletedIds.add(tag[1]);
      }
    }
  }

  // 2. Build edit map: targetId → latest edit content.
  final edits = <String, _Edit>{};
  for (final event in events) {
    if (event.kind != EventKind.streamMessageEdit) continue;
    if (deletedIds.contains(event.id)) continue;

    final targetId = _lastETag(event.tags);
    if (targetId == null || deletedIds.contains(targetId)) continue;

    final existing = edits[targetId];
    if (existing == null || event.createdAt > existing.createdAt) {
      edits[targetId] = _Edit(
        content: event.content,
        createdAt: event.createdAt,
        tags: event.tags,
      );
    }
  }

  // 3. Aggregate reactions: targetId → { emoji → { pubkey → eventId } }.
  final reactionMap = <String, Map<String, Map<String, String>>>{};
  final reactionEmojiUrls = <String, Map<String, String>>{};
  for (final event in events) {
    if (event.kind != EventKind.reaction) continue;
    if (deletedIds.contains(event.id)) continue;

    final targetId = _lastETag(event.tags);
    if (targetId == null || deletedIds.contains(targetId)) continue;

    final emoji = event.content.trim();
    if (emoji.isEmpty) continue;

    reactionMap
            .putIfAbsent(targetId, () => {})
            .putIfAbsent(emoji, () => {})[event.pubkey.toLowerCase()] =
        event.id;

    final shortcode = normalizeShortcode(emoji);
    if (shortcode != null) {
      for (final tag in event.tags) {
        if (tag.length < 3 || tag[0] != 'emoji') continue;
        if (normalizeShortcode(tag[1]) == shortcode) {
          reactionEmojiUrls.putIfAbsent(targetId, () => {})[emoji] = tag[2];
          break;
        }
      }
    }
  }

  final normalizedCurrentPubkey = currentPubkey?.toLowerCase();

  List<TimelineReaction> reactionsFor(String eventId) {
    final emojiMap = reactionMap[eventId];
    if (emojiMap == null) return const [];

    return [
      for (final entry in emojiMap.entries)
        TimelineReaction(
          emoji: entry.key,
          count: entry.value.length,
          reactedByCurrentUser:
              normalizedCurrentPubkey != null &&
              entry.value.containsKey(normalizedCurrentPubkey),
          userPubkeys: entry.value.keys.toList(),
          emojiUrl: reactionEmojiUrls[eventId]?[entry.key],
          currentUserReactionId: normalizedCurrentPubkey != null
              ? entry.value[normalizedCurrentPubkey]
              : null,
        ),
    ];
  }

  // 4. Filter to visible content events and build TimelineMessages.
  final result = <TimelineMessage>[];
  for (final event in events) {
    if (deletedIds.contains(event.id)) continue;

    if (event.kind == EventKind.systemMessage) {
      final systemEvent = SystemEvent.fromContent(event.content);
      if (systemEvent != null) {
        result.add(
          TimelineMessage(
            id: event.id,
            pubkey: event.pubkey,
            createdAt: event.createdAt,
            content: event.content,
            tags: event.tags,
            isSystem: true,
            systemEvent: systemEvent,
            reactions: reactionsFor(event.id),
          ),
        );
      }
      continue;
    }

    if (event.kind == EventKind.huddleStarted ||
        event.kind == EventKind.huddleEnded) {
      final systemEvent = SystemEvent.fromHuddleEvent(event);
      if (systemEvent != null) {
        result.add(
          TimelineMessage(
            id: event.id,
            pubkey: event.pubkey,
            createdAt: event.createdAt,
            content: event.content,
            tags: event.tags,
            isSystem: true,
            systemEvent: systemEvent,
            reactions: reactionsFor(event.id),
          ),
        );
      }
      continue;
    }

    if (event.kind == EventKind.streamMessage ||
        event.kind == EventKind.streamMessageV2 ||
        event.kind == EventKind.streamMessageDiff) {
      final edit = edits[event.id];
      final effectiveTags = edit?.tags ?? event.tags;
      // Include both notify (`p`) and reference-only (`mention`) tags —
      // mirrors desktop's resolveMentionNames, so names in messages sent
      // "without inviting" still render as mentions.
      final mentions = <String>[
        for (final tag in effectiveTags)
          if (tag.length >= 2 && (tag[0] == 'p' || tag[0] == 'mention')) tag[1],
      ];

      final threadRef = event.threadReference;

      result.add(
        TimelineMessage(
          id: event.id,
          pubkey: event.pubkey,
          createdAt: event.createdAt,
          content: edit?.content ?? event.content,
          tags: effectiveTags,
          edited: edit != null,
          mentionPubkeys: mentions,
          reactions: reactionsFor(event.id),
          parentId: threadRef.parentId,
          rootId: threadRef.rootId,
        ),
      );
    }
  }

  return result;
}

/// Build main-timeline entries: only root messages (parentId == null),
/// each with an optional [ThreadSummary] when replies exist.
///
/// Mirrors the desktop's `buildMainTimelineEntries`.
List<MainTimelineEntry> buildMainTimelineEntries(
  List<TimelineMessage> messages, {
  Map<String, ChannelWindowThreadSummary>? relaySummaries,
}) {
  // Index descendant stats by ancestor, so a nested reply updates every summary
  // above it and not only the summary of its direct parent.
  final descendantStats = _buildDescendantStats(messages);

  return [
    for (final msg in messages)
      if (msg.parentId == null || _isBroadcastReply(msg))
        MainTimelineEntry(
          message: msg,
          summary: _buildSummary(
            msg.id,
            descendantStats,
            relaySummaries?[msg.id],
          ),
        ),
  ];
}

bool _isBroadcastReply(TimelineMessage message) {
  return message.tags.any(
    (tag) => tag.length >= 2 && tag[0] == 'broadcast' && tag[1] == '1',
  );
}

/// Combine what the relay counted with what this client has actually seen.
///
/// The count and last-reply time follow the desktop's `mergeThreadSummaries`
/// (`desktop/src/features/messages/lib/threadPanel.ts`): the relay recount is
/// authoritative for replies this client never loaded, and the locally observed
/// replies are authoritative for anything that landed after (or alongside) the
/// last recount. Neither source alone is complete, so take the larger count and
/// the later reply time rather than letting one shadow the other. The facepile
/// order is mobile's own (see [_mergeParticipants]) because this file already
/// renders relay participants in the order the relay sent them.
///
/// Both halves count descendants, not direct replies: the relay's
/// `descendant_count` and the locally assembled [_buildDescendantStats]. A badge
/// on the main timeline stands for the whole thread under that message, so a
/// reply to a reply has to raise it.
ThreadSummary? _buildSummary(
  String messageId,
  Map<String, _DescendantStats> descendantStats,
  ChannelWindowThreadSummary? relaySummary,
) {
  final local = _buildLocalSummary(messageId, descendantStats);
  final relay = _buildRelaySummary(messageId, relaySummary);
  if (relay == null) return local;
  if (local == null) return relay;

  return ThreadSummary(
    threadHeadId: messageId,
    replyCount: local.replyCount > relay.replyCount
        ? local.replyCount
        : relay.replyCount,
    // Relay participants first: they describe the whole thread, including
    // replies this client never loaded, so a recount's facepile keeps rendering
    // as it does today. Locally seen repliers (newest first, matching the relay
    // order this file already renders) only fill the remaining slots.
    participantPubkeys: _mergeParticipants(
      relay.participantPubkeys,
      local.participantPubkeys.reversed,
    ),
    lastReplyAt: _laterOf(local.lastReplyAt, relay.lastReplyAt),
  );
}

/// Summary assembled from the replies present in the loaded timeline.
///
/// Counts every loaded descendant, not only direct children, because the root
/// badge in the main timeline stands for the whole thread. This mirrors the
/// desktop's `buildSummaryForDirectReplies`, which reads the same descendant
/// stats and reverses the newest-first participants to oldest-first.
ThreadSummary? _buildLocalSummary(
  String messageId,
  Map<String, _DescendantStats> descendantStats,
) {
  final stats = descendantStats[messageId];
  if (stats == null || stats.descendantCount == 0) return null;

  return ThreadSummary(
    threadHeadId: messageId,
    replyCount: stats.descendantCount,
    participantPubkeys: stats.recentParticipantsNewestFirst.reversed.toList(),
    lastReplyAt: stats.lastReplyAt,
  );
}

/// Descendant count, last reply time, and recent participants for every message
/// that has at least one loaded descendant.
///
/// Mirrors the desktop's `buildDescendantStatsByMessageId`
/// (`desktop/src/features/messages/lib/threadPanel.ts`): each message is
/// attributed to every ancestor on its parent chain, so a reply nested under a
/// reply still counts towards the root it belongs to. Messages are visited
/// newest first so the capped participant list keeps the most recent repliers.
Map<String, _DescendantStats> _buildDescendantStats(
  List<TimelineMessage> messages,
) {
  final messageById = <String, TimelineMessage>{
    for (final msg in messages) msg.id: msg,
  };
  final statsByMessageId = <String, _DescendantStats>{
    for (final msg in messages) msg.id: _DescendantStats(),
  };

  // Oldest first, keeping the original order for messages sharing a timestamp,
  // then walked in reverse so participants are collected newest first.
  final ordered = List<int>.generate(messages.length, (index) => index)
    ..sort((left, right) {
      final byCreatedAt = messages[left].createdAt.compareTo(
        messages[right].createdAt,
      );
      return byCreatedAt != 0 ? byCreatedAt : left.compareTo(right);
    });

  for (var i = ordered.length - 1; i >= 0; i--) {
    final message = messages[ordered[i]];
    final participant = message.pubkey.toLowerCase();

    // Cap the walk so a malformed parent chain (a cycle, for instance) cannot
    // spin forever.
    var ancestorId = message.parentId;
    var hops = 0;
    final maxHops = messages.length + 1;

    while (ancestorId != null && hops < maxHops) {
      final ancestorStats = statsByMessageId[ancestorId];
      if (ancestorStats == null) break;

      ancestorStats.descendantCount += 1;
      ancestorStats.lastReplyAt = _laterOf(
        ancestorStats.lastReplyAt,
        message.createdAt,
      );
      if (ancestorStats.recentParticipantsNewestFirst.length < 3 &&
          !ancestorStats.recentParticipantsNewestFirst.contains(participant)) {
        ancestorStats.recentParticipantsNewestFirst.add(participant);
      }

      ancestorId = messageById[ancestorId]?.parentId;
      hops += 1;
    }
  }

  return statsByMessageId;
}

/// Mutable accumulator for [_buildDescendantStats].
class _DescendantStats {
  int descendantCount = 0;
  int? lastReplyAt;
  final List<String> recentParticipantsNewestFirst = [];
}

/// Summary from the relay's recount, or null when it reports no replies.
ThreadSummary? _buildRelaySummary(
  String messageId,
  ChannelWindowThreadSummary? relaySummary,
) {
  if (relaySummary == null || relaySummary.descendantCount <= 0) return null;
  return ThreadSummary(
    threadHeadId: messageId,
    replyCount: relaySummary.descendantCount,
    participantPubkeys: relaySummary.participantPubkeys.take(3).toList(),
    lastReplyAt: relaySummary.lastReplyAt,
  );
}

int? _laterOf(int? left, int? right) {
  if (left == null) return right;
  if (right == null) return left;
  return left > right ? left : right;
}

/// Up to 3 unique pubkeys, [primary] first.
///
/// [secondary] is expected newest-first so that a capped facepile keeps the
/// most recent participants rather than the oldest ones. Uniqueness is
/// case-insensitive because the locally assembled half lowercases pubkeys while
/// the relay half is passed through as received.
List<String> _mergeParticipants(
  Iterable<String> primary,
  Iterable<String> secondary,
) {
  final seen = <String>{};
  final merged = <String>[];
  for (final pubkey in [...primary, ...secondary]) {
    if (!seen.add(pubkey.toLowerCase())) continue;
    merged.add(pubkey);
    if (merged.length == 3) break;
  }
  return merged;
}

class _Edit {
  final String content;
  final int createdAt;
  final List<List<String>> tags;

  const _Edit({
    required this.content,
    required this.createdAt,
    required this.tags,
  });
}

/// Get the last `e` tag value (reaction/edit target convention).
String? _lastETag(List<List<String>> tags) {
  for (var i = tags.length - 1; i >= 0; i--) {
    final tag = tags[i];
    if (tag.length >= 2 && tag[0] == 'e') return tag[1];
  }
  return null;
}

String? _readString(Map<dynamic, dynamic> json, String key) {
  final value = json[key];
  return value is String ? value : null;
}
