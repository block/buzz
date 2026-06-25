import 'dart:convert';

import 'package:flutter/foundation.dart';

/// Nostr event kind constants.
///
/// Keep in sync with `desktop/src/shared/constants/kinds.ts`.
abstract final class EventKind {
  static const note = 1;
  static const contactList = 3;
  static const deletion = 5;
  static const reaction = 7;
  static const streamMessage = 9;
  static const nip29DeleteEvent = 9005;
  static const presenceUpdate = 20001;
  static const typingIndicator = 20002;
  static const auth = 22242;
  static const agentObserverFrame = 24200;
  static const readState = 30078;
  static const userStatus = 30315;
  static const streamMessageV2 = 40002;
  static const streamMessageEdit = 40003;
  static const streamMessageDiff = 40008;
  static const systemMessage = 40099;
  static const forumPost = 45001;
  static const forumComment = 45003;

  /// Event kinds that represent user-visible channel messages.
  static const channelMessageEventKinds = [
    streamMessage, // 9
    streamMessageV2, // 40002
    forumPost, // 45001
    forumComment, // 45003
  ];

  /// Event kinds that represent channel activity (messages, edits, reactions,
  /// deletions, system events). Matches the desktop's `CHANNEL_EVENT_KINDS`.
  static const channelEventKinds = [
    deletion, // 5
    reaction, // 7
    nip29DeleteEvent, // 9005 — Buzz-native deletion
    ...channelMessageEventKinds,
    40001, // legacy pre-migration stream messages
    streamMessageEdit, // 40003
    streamMessageDiff, // 40008
    systemMessage, // 40099
  ];
}

/// A Nostr event as defined by NIP-01.
@immutable
class NostrEvent {
  final String id;
  final String pubkey;
  final int createdAt;
  final int kind;
  final List<List<String>> tags;
  final String content;
  final String sig;

  const NostrEvent({
    required this.id,
    required this.pubkey,
    required this.createdAt,
    required this.kind,
    required this.tags,
    required this.content,
    required this.sig,
  });

  factory NostrEvent.fromJson(Map<String, dynamic> json) {
    return NostrEvent(
      id: json['id'] as String,
      pubkey: json['pubkey'] as String,
      createdAt: json['created_at'] as int,
      kind: json['kind'] as int,
      tags: (json['tags'] as List<dynamic>)
          .map((t) => (t as List<dynamic>).map((e) => e as String).toList())
          .toList(),
      content: json['content'] as String,
      sig: json['sig'] as String,
    );
  }

  Map<String, dynamic> toJson() => {
    'id': id,
    'pubkey': pubkey,
    'created_at': createdAt,
    'kind': kind,
    'tags': tags,
    'content': content,
    'sig': sig,
  };

  /// Get the first value for a given tag key.
  String? getTagValue(String key) {
    for (final tag in tags) {
      if (tag.isNotEmpty && tag[0] == key && tag.length > 1) {
        return tag[1];
      }
    }
    return null;
  }

  /// The channel/group ID from the `h` tag (NIP-29).
  String? get channelId => getTagValue('h');

  /// Extract thread parent and root IDs from `e` tags.
  ///
  /// Matches the desktop's `getThreadReference` logic:
  /// - Tags with marker `"reply"` identify the direct parent.
  /// - Tags with marker `"root"` identify the thread root.
  /// - If no markers are present, falls back to null (top-level message).
  ({String? parentId, String? rootId}) get threadReference {
    final eTags = [
      for (final tag in tags)
        if (tag.length >= 2 && tag[0] == 'e') tag,
    ];

    if (eTags.isEmpty) return (parentId: null, rootId: null);

    // Find tagged root and reply markers (desktop convention).
    List<String>? rootTag;
    List<String>? replyTag;
    for (final tag in eTags) {
      if (tag.length >= 4) {
        if (tag[3] == 'root') rootTag = tag;
        if (tag[3] == 'reply') replyTag = tag;
      }
    }

    if (replyTag == null) return (parentId: null, rootId: null);

    final parentId = replyTag[1];
    final rootId = rootTag?[1] ?? parentId;
    return (parentId: parentId, rootId: rootId);
  }

  /// The parent event ID from the `e` tag.
  String? get parentEventId => threadReference.parentId;

  @override
  bool operator ==(Object other) =>
      identical(this, other) || other is NostrEvent && id == other.id;

  @override
  int get hashCode => id.hashCode;
}

/// A NIP-01 subscription filter.
@immutable
class NostrFilter {
  final List<int> kinds;
  final List<String>? authors;

  /// Specific event IDs (NIP-01 single-event lookup).
  final List<String>? ids;
  final int limit;
  final int? since;
  final int? until;

  /// NIP-50 full-text search query.
  final String? search;

  /// Tag filters, e.g. `{'#h': ['channel-id']}`.
  final Map<String, List<String>> tags;

  const NostrFilter({
    required this.kinds,
    this.authors,
    this.ids,
    this.limit = 100,
    this.since,
    this.until,
    this.search,
    this.tags = const {},
  });

  /// Return a copy with an updated `since` value.
  NostrFilter copyWithSince(int since) => NostrFilter(
    kinds: kinds,
    authors: authors,
    ids: ids,
    limit: limit,
    since: since,
    until: until,
    search: search,
    tags: tags,
  );

  Map<String, dynamic> toJson() {
    final json = <String, dynamic>{'kinds': kinds, 'limit': limit};
    if (authors != null) json['authors'] = authors;
    if (ids != null) json['ids'] = ids;
    if (since != null) json['since'] = since;
    if (until != null) json['until'] = until;
    if (search != null) json['search'] = search;
    for (final entry in tags.entries) {
      json[entry.key] = entry.value;
    }
    return json;
  }
}

/// Parsed kind:0 user profile metadata.
@immutable
class ProfileData {
  final String pubkey;
  final String? displayName;
  final String? avatarUrl;
  final String? about;
  final String? nip05;

  const ProfileData({
    required this.pubkey,
    this.displayName,
    this.avatarUrl,
    this.about,
    this.nip05,
  });

  factory ProfileData.fromEvent(NostrEvent event) {
    Map<String, dynamic> meta = {};
    try {
      final decoded = jsonDecode(event.content);
      if (decoded is Map<String, dynamic>) meta = decoded;
    } catch (_) {}
    return ProfileData(
      pubkey: event.pubkey,
      displayName:
          (meta['display_name'] as String?) ?? (meta['name'] as String?),
      avatarUrl: meta['picture'] as String?,
      about: meta['about'] as String?,
      nip05: meta['nip05'] as String?,
    );
  }
}

/// Parsed kind:39000 channel metadata.
@immutable
class ChannelData {
  final String id;
  final String name;
  final String channelType;
  final String visibility;
  final String description;
  final String? topic;
  final List<String> participantPubkeys;
  final int? ttlSeconds;
  final DateTime? ttlDeadline;
  final bool isArchived;

  const ChannelData({
    required this.id,
    required this.name,
    required this.channelType,
    required this.visibility,
    required this.description,
    this.topic,
    this.participantPubkeys = const [],
    this.ttlSeconds,
    this.ttlDeadline,
    this.isArchived = false,
  });

  factory ChannelData.fromEvent(NostrEvent event) {
    final id = event.getTagValue('d') ?? '';
    final name = event.getTagValue('name') ?? '';
    // Prefer explicit ["t", type]; fall back to ["hidden"] => dm, else "stream".
    // The fallback exists for relays that haven't been upgraded to emit the
    // explicit type tag yet.
    final explicitType = event.getTagValue('t');
    final hasHidden = event.tags.any((t) => t.isNotEmpty && t[0] == 'hidden');
    final channelType = explicitType ?? (hasHidden ? 'dm' : 'stream');
    // Prefer explicit ["public"]; fall back to NIP-29 absence-of-"private".
    final hasPublic = event.tags.any((t) => t.isNotEmpty && t[0] == 'public');
    final hasPrivate = event.tags.any((t) => t.isNotEmpty && t[0] == 'private');
    final visibility = hasPublic
        ? 'open'
        : hasPrivate
        ? 'private'
        : 'open';
    final description = event.getTagValue('about') ?? '';
    final topic = event.getTagValue('topic');
    final participants = [
      for (final t in event.tags)
        if (t.length >= 2 && t[0] == 'p') t[1],
    ];
    final ttlRaw = event.getTagValue('ttl');
    final ttlSeconds = ttlRaw != null ? int.tryParse(ttlRaw) : null;
    final ttlDeadlineRaw = event.getTagValue('ttl_deadline');
    final ttlDeadline = ttlDeadlineRaw != null
        ? DateTime.tryParse(ttlDeadlineRaw)
        : null;
    // Relay republishes kind:39000 with `["archived", "true"]` when a channel
    // is archived (including the auto-archive emitted by the TTL reaper). The
    // tag value "false" is also accepted server-side, so only treat "true" as
    // archived — anything else (missing tag, "false", unexpected value) means
    // active.
    final isArchived = event.getTagValue('archived') == 'true';
    return ChannelData(
      id: id,
      name: name,
      channelType: channelType,
      visibility: visibility,
      description: description,
      topic: topic,
      participantPubkeys: participants,
      ttlSeconds: ttlSeconds,
      ttlDeadline: ttlDeadline,
      isArchived: isArchived,
    );
  }
}

/// A single member entry parsed from a kind:39002 members event.
@immutable
class MemberEntry {
  final String pubkey;
  final String role;

  const MemberEntry({required this.pubkey, required this.role});
}

/// Parse a kind:39002 members event into the list of `(pubkey, role)` entries.
///
/// NIP-29 members tags follow the shape `["p", <pubkey>, <relay>, <role>]`.
List<MemberEntry> membersFromEvent(NostrEvent event) {
  return [
    for (final t in event.tags)
      if (t.length >= 2 && t[0] == 'p')
        MemberEntry(pubkey: t[1], role: t.length >= 4 ? t[3] : 'member'),
  ];
}

/// Parse a Buzz command response from the relay's OK message content.
///
/// Command kinds (e.g. 41010, 30620, 46020) return `"response:{...}"` in the
/// OK message. Returns `null` if the message is not a command response or the
/// JSON is invalid.
Map<String, dynamic>? parseCommandResponse(String message) {
  // Try the spec format first: "response:{...}".
  const prefix = 'response:';
  if (message.startsWith(prefix)) {
    try {
      final decoded = jsonDecode(message.substring(prefix.length));
      if (decoded is Map<String, dynamic>) return decoded;
    } catch (_) {}
    return null;
  }
  // Fallback: raw JSON object (older relays, backward compat).
  try {
    final decoded = jsonDecode(message);
    if (decoded is Map<String, dynamic>) return decoded;
  } catch (_) {}
  return null;
}
