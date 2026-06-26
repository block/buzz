import 'dart:convert';

import 'package:flutter/foundation.dart';

import '../../shared/relay/relay.dart';
import '../custom_emoji/custom_emoji.dart';

enum SystemEventType {
  memberJoined,
  memberLeft,
  memberRemoved,
  topicChanged,
  purposeChanged,
  channelCreated,
  channelArchived,
  channelUnarchived,
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
        return '$actor added $target to the channel';
      }(),
      SystemEventType.memberLeft => '$actor left the channel',
      SystemEventType.memberRemoved => () {
        final target = resolveLabel(targetPubkey);
        return '$actor removed $target from the channel';
      }(),
      SystemEventType.topicChanged => '$actor changed the topic to "$topic"',
      SystemEventType.purposeChanged =>
        '$actor changed the purpose to "$purpose"',
      SystemEventType.channelCreated => '$actor created this channel',
      SystemEventType.channelArchived => '$actor archived this channel',
      SystemEventType.channelUnarchived => '$actor unarchived this channel',
    };
  }
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
}

@immutable
class ThreadSummary {
  final String threadHeadId;
  final int replyCount;

  /// Up to 3 most recent unique participant pubkeys.
  final List<String> participantPubkeys;

  const ThreadSummary({
    required this.threadHeadId,
    required this.replyCount,
    required this.participantPubkeys,
  });
}

/// A main-timeline entry: a root message with an optional thread summary.
@immutable
class MainTimelineEntry {
  final TimelineMessage message;
  final ThreadSummary? summary;

  const MainTimelineEntry({required this.message, this.summary});
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

  // 4. Filter to visible content events and build TimelineMessages.
  final result = <TimelineMessage>[];
  for (final event in events) {
    if (deletedIds.contains(event.id)) continue;

    if (event.kind == EventKind.systemMessage) {
      final systemEvent = SystemEvent.fromContent(event.content);
      if (systemEvent != null) {
        final emojiMap = reactionMap[event.id];
        final reactions = <TimelineReaction>[
          if (emojiMap != null)
            for (final entry in emojiMap.entries)
              TimelineReaction(
                emoji: entry.key,
                count: entry.value.length,
                reactedByCurrentUser:
                    normalizedCurrentPubkey != null &&
                    entry.value.containsKey(normalizedCurrentPubkey),
                userPubkeys: entry.value.keys.toList(),
                emojiUrl: reactionEmojiUrls[event.id]?[entry.key],
                currentUserReactionId: normalizedCurrentPubkey != null
                    ? entry.value[normalizedCurrentPubkey]
                    : null,
              ),
        ];

        result.add(
          TimelineMessage(
            id: event.id,
            pubkey: event.pubkey,
            createdAt: event.createdAt,
            content: event.content,
            tags: event.tags,
            isSystem: true,
            systemEvent: systemEvent,
            reactions: reactions,
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
      final mentions = <String>[
        for (final tag in effectiveTags)
          if (tag.length >= 2 && tag[0] == 'p') tag[1],
      ];

      final emojiMap = reactionMap[event.id];
      final reactions = <TimelineReaction>[
        if (emojiMap != null)
          for (final entry in emojiMap.entries)
            TimelineReaction(
              emoji: entry.key,
              count: entry.value.length,
              reactedByCurrentUser:
                  normalizedCurrentPubkey != null &&
                  entry.value.containsKey(normalizedCurrentPubkey),
              userPubkeys: entry.value.keys.toList(),
              emojiUrl: reactionEmojiUrls[event.id]?[entry.key],
              currentUserReactionId: normalizedCurrentPubkey != null
                  ? entry.value[normalizedCurrentPubkey]
                  : null,
            ),
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
          reactions: reactions,
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
  List<TimelineMessage> messages,
) {
  // Index direct children by parentId.
  final childrenByParent = <String, List<TimelineMessage>>{};
  for (final msg in messages) {
    final pid = msg.parentId;
    if (pid == null) continue;
    childrenByParent.putIfAbsent(pid, () => []).add(msg);
  }

  return [
    for (final msg in messages)
      if (msg.parentId == null || _isBroadcastReply(msg))
        MainTimelineEntry(
          message: msg,
          summary: _buildSummary(msg.id, childrenByParent),
        ),
  ];
}

bool _isBroadcastReply(TimelineMessage message) {
  return message.tags.any(
    (tag) => tag.length >= 2 && tag[0] == 'broadcast' && tag[1] == '1',
  );
}

ThreadSummary? _buildSummary(
  String messageId,
  Map<String, List<TimelineMessage>> childrenByParent,
) {
  final replies = childrenByParent[messageId];
  if (replies == null || replies.isEmpty) return null;

  // Up to 3 most recent unique participants (walk backwards).
  final seen = <String>{};
  final participants = <String>[];
  for (var i = replies.length - 1; i >= 0 && participants.length < 3; i--) {
    final pk = replies[i].pubkey.toLowerCase();
    if (seen.add(pk)) participants.add(pk);
  }

  return ThreadSummary(
    threadHeadId: messageId,
    replyCount: replies.length,
    participantPubkeys: participants.reversed.toList(),
  );
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
