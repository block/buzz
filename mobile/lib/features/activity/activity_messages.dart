import '../../shared/relay/relay.dart';

/// Message kinds eligible as Activity conversation and ownership evidence.
const activityMessageKinds = [9, 40002, 40008, 45001, 45003];

/// Project validated edits/deletions without depending on a channel UI model.
Iterable<NostrEvent> activityMessages(
  Iterable<NostrEvent> events,
  List<NostrEvent> updates,
) sync* {
  final deleted = {
    for (final update in updates)
      if (update.kind == 5 || update.kind == 9005)
        for (final tag in update.tags)
          if (tag.length > 1 && tag[0] == 'e') tag[1],
  };
  final edits = <String, NostrEvent>{};
  for (final edit in updates) {
    if (edit.kind != 40003 || deleted.contains(edit.id)) continue;
    final targets = edit.tags.where((t) => t.length > 1 && t[0] == 'e');
    if (targets.isEmpty) continue;
    final target = targets.last[1];
    if (edits[target] == null || edit.createdAt > edits[target]!.createdAt) {
      edits[target] = edit;
    }
  }
  for (final event in events) {
    if (!activityMessageKinds.contains(event.kind) ||
        deleted.contains(event.id)) {
      continue;
    }
    final edit = edits[event.id];
    yield edit == null
        ? event
        : NostrEvent(
            id: event.id,
            pubkey: event.pubkey,
            createdAt: event.createdAt,
            kind: event.kind,
            content: edit.content,
            tags: edit.tags,
            sig: event.sig,
          );
  }
}
