import 'package:flutter/foundation.dart';

const Object _sentinel = Object();

@immutable
class Channel {
  final String id;
  final String name;
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

  bool get isEphemeral => ttlSeconds != null || ttlDeadline != null;

  bool get isStream => channelType == 'stream';
  bool get isForum => channelType == 'forum';
  bool get isDm => channelType == 'dm';
  bool get isPrivate => visibility == 'private';
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
    Object? lastMessageAt = _sentinel,
    Object? archivedAt = _sentinel,
    int? memberCount,
    bool? isMember,
  }) => Channel(
    id: id,
    name: name,
    channelType: channelType,
    visibility: visibility,
    description: description,
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
