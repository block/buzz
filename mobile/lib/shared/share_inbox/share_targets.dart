import 'package:flutter/foundation.dart';

/// One destination the iOS Share Extension may offer.
///
/// Mirrors `BuzzShareTargets.Target` in `ios/BuzzShareInbox/BuzzShareInbox.swift`.
@immutable
class ShareTarget {
  final String channelId;
  final String label;
  final bool isDm;
  final DateTime? lastMessageAt;

  const ShareTarget({
    required this.channelId,
    required this.label,
    required this.isDm,
    this.lastMessageAt,
  });

  Map<String, Object?> toArguments() => {
    'channelId': channelId,
    'label': label,
    'isDm': isDm,
    'lastMessageAt': lastMessageAt == null
        ? null
        : lastMessageAt!.millisecondsSinceEpoch / 1000,
  };

  @override
  bool operator ==(Object other) =>
      other is ShareTarget &&
      other.channelId == channelId &&
      other.label == label &&
      other.isDm == isDm &&
      other.lastMessageAt == lastMessageAt;

  @override
  int get hashCode => Object.hash(channelId, label, isDm, lastMessageAt);
}

@immutable
class ShareTargetCommunity {
  final String id;
  final String name;

  const ShareTargetCommunity({required this.id, required this.name});

  Map<String, Object?> toArguments() => {'id': id, 'name': name};
}

/// The active community's destinations plus the full community list, as
/// published to the extension. Other communities' targets are kept by the
/// native side from their own last publication.
@immutable
class ShareTargetsSnapshot {
  final String communityId;
  final List<ShareTargetCommunity> communities;
  final List<ShareTarget> targets;

  const ShareTargetsSnapshot({
    required this.communityId,
    required this.communities,
    required this.targets,
  });

  Map<String, Object?> toArguments() => {
    'communityId': communityId,
    'communities': [
      for (final community in communities) community.toArguments(),
    ],
    'targets': [for (final target in targets) target.toArguments()],
  };

  /// Cheap identity for skipping republication of an unchanged snapshot.
  String get fingerprint => [
    communityId,
    for (final community in communities) '${community.id}=${community.name}',
    for (final target in targets)
      '${target.channelId}|${target.label}|${target.isDm}|'
          '${target.lastMessageAt?.millisecondsSinceEpoch}',
  ].join('\u0000');
}
