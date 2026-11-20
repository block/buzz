import '../../shared/relay/relay.dart';

final _eventId = RegExp(r'^[0-9a-fA-F]{64}$');

/// Requires the rendered edit target to identify the relay-authorized target.
/// Relay edit authorization selects the first valid hex e-tag in signed order;
/// the formatter selects the last e-tag, including non-target references.
bool forumEditTargetsAgree(NostrEvent event) {
  if (event.kind != EventKind.streamMessageEdit) return true;
  String? first;
  String? last;
  for (final tag in event.tags) {
    if (tag.length < 2 || tag[0] != 'e') continue;
    last = tag[1];
    if (first == null && last.length == 64 && _eventId.hasMatch(last)) {
      first = last.toLowerCase();
    }
  }
  return first != null && first == last?.toLowerCase();
}

/// Checks effects without relay authority against known targets' authors.
/// Unknown deletion references have no effect on known targets; edits apply
/// to their last e-tag and require that target's author to be known.
bool forumAuxiliaryAuthorMatches(
  NostrEvent event,
  Map<String, NostrEvent> targets,
) {
  final edit = event.kind == EventKind.streamMessageEdit;
  if (!edit &&
      event.kind != EventKind.deletion &&
      event.kind != EventKind.nip29DeleteEvent) {
    return true;
  }
  final references = event.tags
      .where((tag) => tag.length >= 2 && tag[0] == 'e')
      .map((tag) => tag[1]);
  for (final id in edit ? references.toList().reversed.take(1) : references) {
    if (!edit && !targets.containsKey(id)) continue;
    if (targets[id]?.pubkey != event.pubkey) return false;
  }
  return true;
}

/// Captures the known deletion references authorized on this fetched page.
Set<String> forumKnownDeletionTargets(
  NostrEvent event,
  Set<String> knownIds,
) => {
  for (final tag in event.tags)
    if (tag.length >= 2 && tag[0] == 'e' && knownIds.contains(tag[1])) tag[1],
};
