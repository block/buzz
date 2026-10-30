import 'package:flutter/foundation.dart';

const Object _sentinel = Object();

/// Shown when a private-channel add is refused, so a missing Invite action
/// reads as a rule rather than a bug.
const privateChannelAddDeniedMessage =
    'Only channel members can add people to a private channel.';

@immutable
class Channel {
  final String id;
  final String name;

  /// NIP-CL labels from metadata; presentation hints, never access grants.
  final List<String> labels;
  final String channelType; // "stream", "forum", "dm"
  final String visibility; // "open", "private"
  final String description;
  final String? topic;
  final String? purpose;
  final String createdBy;
  final DateTime createdAt;
  final int memberCount;
  final DateTime? lastMessageAt;
  final DateTime? archivedAt;
  final List<String> participants;
  final List<String> participantPubkeys;
  final bool isMember;
  final int? ttlSeconds;
  final DateTime? ttlDeadline;

  const Channel({
    required this.id,
    required this.name,
    required this.channelType,
    this.labels = const [],
    required this.visibility,
    required this.description,
    required this.createdBy,
    required this.createdAt,
    required this.memberCount,
    this.topic,
    this.purpose,
    this.lastMessageAt,
    this.archivedAt,
    this.participants = const [],
    this.participantPubkeys = const [],
    this.isMember = false,
    this.ttlSeconds,
    this.ttlDeadline,
  });

  factory Channel.fromJson(Map<String, dynamic> json) => Channel(
    id: json['id'] as String,
    name: json['name'] as String,
    channelType: json['channel_type'] as String,
    labels: (json['labels'] as List<dynamic>? ?? const []).cast<String>(),
    visibility: json['visibility'] as String,
    description: (json['description'] as String?) ?? '',
    topic: json['topic'] as String?,
    purpose: json['purpose'] as String?,
    createdBy: json['created_by'] as String,
    createdAt: DateTime.parse(json['created_at'] as String),
    memberCount: json['member_count'] as int,
    lastMessageAt: json['last_message_at'] != null
        ? DateTime.parse(json['last_message_at'] as String)
        : null,
    archivedAt: json['archived_at'] != null
        ? DateTime.parse(json['archived_at'] as String)
        : null,
    participants: (json['participants'] as List<dynamic>? ?? const [])
        .cast<String>(),
    participantPubkeys:
        (json['participant_pubkeys'] as List<dynamic>? ?? const [])
            .cast<String>(),
    isMember: json['is_member'] as bool? ?? false,
    ttlSeconds: json['ttl_seconds'] as int?,
    ttlDeadline: json['ttl_deadline'] != null
        ? DateTime.parse(json['ttl_deadline'] as String)
        : null,
  );

  /// Inverse of [Channel.fromJson].
  Map<String, dynamic> toJson() => {
    'id': id,
    'name': name,
    'channel_type': channelType,
    'labels': labels,
    'visibility': visibility,
    'description': description,
    'topic': topic,
    'purpose': purpose,
    'created_by': createdBy,
    'created_at': createdAt.toIso8601String(),
    'member_count': memberCount,
    'last_message_at': lastMessageAt?.toIso8601String(),
    'archived_at': archivedAt?.toIso8601String(),
    'participants': participants,
    'participant_pubkeys': participantPubkeys,
    'is_member': isMember,
    'ttl_seconds': ttlSeconds,
    'ttl_deadline': ttlDeadline?.toIso8601String(),
  };

  bool get isEphemeral => ttlSeconds != null || ttlDeadline != null;

  /// Solo presentation is carried by NIP-CL metadata, not project filing.
  bool get isSolo => isStream && isPrivate && labels.contains('buzz.solo');

  /// Keep old mobile-created chats visible while older clients migrate.
  bool isPersonalChat(String? viewer, Iterable<String> legacyChannels) => isSolo
      ? viewer != null && createdBy == viewer
      : legacyChannels.contains(id);

  bool get isStream => channelType == 'stream';
  bool get isForum => channelType == 'forum';

  /// Streams, forums and DMs can @mention people outside them. An outside
  /// person who is not added becomes a reference, which does not notify them.
  /// Streams and forums ask on send. Nobody can be added to a DM, so a DM
  /// does not ask.
  bool get mentionsOutsidePeople => isStream || isForum || isDm;
  bool get isDm => channelType == 'dm';
  bool get isPrivate => visibility == 'private';
  bool get canJoin => visibility == 'open' && !isArchived && !isMember && !isDm;

  /// Whether [selfRole] may add *another* identity here, mirroring the relay's
  /// kind:9000 authority (`validate_admin_event` + `add_member`): DMs never,
  /// open channels always, private channels for any active member. Elevated
  /// grants remain reserved for owners/admins. Unknown visibility fails closed.
  bool canAddMembers(String? selfRole) {
    if (isDm) return false;
    if (visibility == 'open') return true;
    if (visibility == 'private') return selfRole != null;
    return false;
  }

  bool get isArchived => archivedAt != null;

  String displayLabel({String? currentPubkey}) {
    if (!isDm || participants.isEmpty) {
      return name;
    }

    final normalizedCurrent = currentPubkey?.toLowerCase();
    final labels = <String>[];
    for (var index = 0; index < participants.length; index++) {
      final participantPubkey = index < participantPubkeys.length
          ? participantPubkeys[index].toLowerCase()
          : null;
      if (participantPubkey != null && participantPubkey == normalizedCurrent) {
        continue;
      }
      labels.add(participants[index]);
    }

    if (labels.isEmpty) {
      labels.addAll(participants);
    }

    return labels.join(', ');
  }

  Channel mergeDetails(ChannelDetails details) => Channel(
    id: id,
    name: details.name,
    channelType: details.channelType,
    labels: details.labels,
    visibility: details.visibility,
    description: details.description,
    topic: details.topic,
    purpose: details.purpose,
    createdBy: details.createdBy,
    createdAt: details.createdAt,
    memberCount: memberCount,
    lastMessageAt: lastMessageAt,
    archivedAt: details.archivedAt,
    participants: participants,
    participantPubkeys: participantPubkeys,
    isMember: isMember,
    ttlSeconds: details.ttlSeconds,
    ttlDeadline: details.ttlDeadline,
  );

  Channel copyWith({
    String? name,
    String? description,
    Object? lastMessageAt = _sentinel,
    Object? archivedAt = _sentinel,
    int? memberCount,
    bool? isMember,
  }) => Channel(
    id: id,
    name: name ?? this.name,
    channelType: channelType,
    labels: labels,
    visibility: visibility,
    description: description ?? this.description,
    topic: topic,
    purpose: purpose,
    createdBy: createdBy,
    createdAt: createdAt,
    memberCount: memberCount ?? this.memberCount,
    lastMessageAt: identical(lastMessageAt, _sentinel)
        ? this.lastMessageAt
        : lastMessageAt as DateTime?,
    archivedAt: identical(archivedAt, _sentinel)
        ? this.archivedAt
        : archivedAt as DateTime?,
    participants: participants,
    participantPubkeys: participantPubkeys,
    isMember: isMember ?? this.isMember,
    ttlSeconds: ttlSeconds,
    ttlDeadline: ttlDeadline,
  );
}

@immutable
class ChannelDetails {
  final String id;
  final String name;

  /// NIP-CL labels from metadata; presentation hints, never access grants.
  final List<String> labels;
  final String channelType;
  final String visibility;
  final String description;
  final String? topic;
  final String? purpose;
  final String createdBy;
  final DateTime createdAt;
  final int memberCount;
  final DateTime? archivedAt;
  final int? ttlSeconds;
  final DateTime? ttlDeadline;

  const ChannelDetails({
    required this.id,
    required this.name,
    required this.channelType,
    this.labels = const [],
    required this.visibility,
    required this.description,
    required this.createdBy,
    required this.createdAt,
    required this.memberCount,
    this.topic,
    this.purpose,
    this.archivedAt,
    this.ttlSeconds,
    this.ttlDeadline,
  });

  factory ChannelDetails.fromJson(Map<String, dynamic> json) => ChannelDetails(
    id: json['id'] as String,
    name: json['name'] as String,
    channelType: json['channel_type'] as String,
    labels: (json['labels'] as List<dynamic>? ?? const []).cast<String>(),
    visibility: json['visibility'] as String,
    description: (json['description'] as String?) ?? '',
    topic: json['topic'] as String?,
    purpose: json['purpose'] as String?,
    createdBy: json['created_by'] as String,
    createdAt: DateTime.parse(json['created_at'] as String),
    memberCount: json['member_count'] as int,
    archivedAt: json['archived_at'] != null
        ? DateTime.parse(json['archived_at'] as String)
        : null,
    ttlSeconds: json['ttl_seconds'] as int?,
    ttlDeadline: json['ttl_deadline'] != null
        ? DateTime.parse(json['ttl_deadline'] as String)
        : null,
  );

  factory ChannelDetails.fromChannel(Channel channel) => ChannelDetails(
    id: channel.id,
    name: channel.name,
    channelType: channel.channelType,
    labels: channel.labels,
    visibility: channel.visibility,
    description: channel.description,
    topic: channel.topic,
    purpose: channel.purpose,
    createdBy: channel.createdBy,
    createdAt: channel.createdAt,
    memberCount: channel.memberCount,
    archivedAt: channel.archivedAt,
    ttlSeconds: channel.ttlSeconds,
    ttlDeadline: channel.ttlDeadline,
  );
}
