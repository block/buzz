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

/// Avatar initial for the DM's visible counterpart.
///
/// Mirrors [resolveDmChannelDisplayLabel]'s participant selection: the
/// first participant that is not [currentPubkey], since member order does
/// not guarantee the counterpart is listed first — otherwise the avatar
/// could identify the current user while the label beside it identifies
/// the counterpart. When every participant is the current user (a
/// self-DM), the label names the current user too, so the avatar keys off
/// that same first participant instead of its label's first character.
///
/// A resolved display name keeps its name-derived initial. A label that
/// fell back to the compact npub form of the participant's key is keyed to
/// the hex public key instead — the npub form starts with `npub1`, so every
/// unnamed participant would otherwise render `N`.
String dmAvatarInitial(Channel channel, {String? currentPubkey}) {
  final normalizedCurrent = currentPubkey?.toLowerCase();
  var index = 0;
  while (normalizedCurrent != null &&
      index < channel.participantPubkeys.length &&
      channel.participantPubkeys[index].toLowerCase() == normalizedCurrent) {
    index++;
  }

  if (index >= channel.participantPubkeys.length) {
    // No participant pubkeys (labels only): fall back to the first
    // participant label, like the channel label does when the non-self
    // list is empty.
    if (channel.participantPubkeys.isEmpty) {
      final label = channel.participants.isNotEmpty
          ? channel.participants.first
          : '';
      return label.isNotEmpty ? label[0].toUpperCase() : '?';
    }
    // Keys exist but every participant is the current user (a self-DM):
    // the label names the current user, so key the avatar to that same
    // first participant with the shared provenance rule below — the hex
    // key for a compact npub label, the authored name otherwise.
    index = 0;
  }

  final pubkey = channel.participantPubkeys[index];
  final label = index < channel.participants.length
      ? channel.participants[index]
      : shortPubkey(pubkey);
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
