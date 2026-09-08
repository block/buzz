import '../../shared/utils/string_utils.dart';
import 'channel.dart';

const int _dmParticipantPreviewLimit = 3;

bool isGenericDmChannelName(String name) {
  final normalized = name.trim().toLowerCase();
  if (normalized.isEmpty ||
      normalized == 'dm' ||
      normalized == 'direct message' ||
      normalized == 'direct messages') {
    return true;
  }
  return RegExp(r'^group dm\s*(\(\d+\))?$').hasMatch(normalized);
}

String formatDmParticipantDisplayName(List<String> displayNames) {
  final visible = displayNames.take(_dmParticipantPreviewLimit).toList();
  final hiddenCount = displayNames.length - visible.length;
  return hiddenCount > 0
      ? [...visible, '+$hiddenCount more'].join(', ')
      : visible.join(', ');
}

String resolveDmChannelDisplayLabel(Channel channel, {String? currentPubkey}) {
  if (!channel.isDm || !isGenericDmChannelName(channel.name)) {
    return channel.name;
  }

  final normalizedCurrent = currentPubkey?.toLowerCase();
  final participants = <({String label, String? pubkey})>[
    for (var index = 0; index < channel.participantPubkeys.length; index++)
      (
        label: index < channel.participants.length
            ? channel.participants[index]
            : shortPubkey(channel.participantPubkeys[index]),
        pubkey: channel.participantPubkeys[index].toLowerCase(),
      ),
  ];

  final displayParticipants = normalizedCurrent == null
      ? participants
      : participants
            .where((participant) => participant.pubkey != normalizedCurrent)
            .toList();
  final labels = <String>{
    for (final participant
        in displayParticipants.isNotEmpty ? displayParticipants : participants)
      participant.label,
  }.toList();

  return labels.isNotEmpty
      ? formatDmParticipantDisplayName(labels)
      : channel.name;
}

/// Avatar initial for the channel's first DM participant label.
///
/// A resolved display name keeps its name-derived initial. A label that
/// fell back to the compact npub form of the participant's key is keyed to
/// the hex public key instead — the npub form starts with `npub1`, so every
/// unnamed participant would otherwise render `N`.
String dmAvatarInitial(Channel channel) {
  if (channel.participants.isEmpty) return '?';
  final label = channel.participants.first;
  final pubkey = channel.participantPubkeys.isNotEmpty
      ? channel.participantPubkeys.first
      : '';
  if (pubkey.isNotEmpty && label == shortPubkey(pubkey)) {
    return pubkey[0].toUpperCase();
  }
  return label.isNotEmpty ? label[0].toUpperCase() : '?';
}

List<Channel> sortDmChannelsByDisplayLabel(
  Iterable<Channel> channels, {
  String? currentPubkey,
}) {
  final sorted = channels.toList();
  sorted.sort((left, right) {
    final leftLabel = resolveDmChannelDisplayLabel(
      left,
      currentPubkey: currentPubkey,
    );
    final rightLabel = resolveDmChannelDisplayLabel(
      right,
      currentPubkey: currentPubkey,
    );
    final labelCompare = leftLabel.toLowerCase().compareTo(
      rightLabel.toLowerCase(),
    );
    if (labelCompare != 0) return labelCompare;
    return left.id.compareTo(right.id);
  });
  return sorted;
}
