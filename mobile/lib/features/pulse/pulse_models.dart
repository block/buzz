import 'package:flutter/foundation.dart';

import '../../shared/relay/nostr_models.dart';

@immutable
class UserNote {
  final String id;
  final String pubkey;
  final int createdAt;
  final String content;
  final List<List<String>> tags;

  const UserNote({
    required this.id,
    required this.pubkey,
    required this.createdAt,
    required this.content,
    required this.tags,
  });

  factory UserNote.fromEvent(NostrEvent event) => UserNote(
    id: event.id,
    pubkey: event.pubkey.toLowerCase(),
    createdAt: event.createdAt,
    content: event.content,
    tags: event.tags,
  );

  String? get replyParentId {
    for (final tag in tags) {
      if (tag.length >= 4 && tag[0] == 'e' && tag[3] == 'reply') return tag[1];
    }
    return null;
  }

  String? get replyParentAuthor {
    for (final tag in tags) {
      if (tag.length >= 2 && tag[0] == 'p') return tag[1].toLowerCase();
    }
    return null;
  }

  List<String> get mentionPubkeys => [
    for (final tag in tags)
      if (tag.length >= 2 && tag[0] == 'p') tag[1].toLowerCase(),
  ];
}

@immutable
class NoteReactionSummary {
  final String noteId;
  final String emoji;
  final int count;
  final List<String> pubkeys;
  final Map<String, String> reactionIdsByPubkey;

  const NoteReactionSummary({
    required this.noteId,
    required this.emoji,
    required this.count,
    required this.pubkeys,
    this.reactionIdsByPubkey = const {},
  });
}

@immutable
class PulseReactionState {
  final int count;
  final bool reactedByCurrentUser;
  final String? currentUserReactionId;

  const PulseReactionState({
    required this.count,
    required this.reactedByCurrentUser,
    this.currentUserReactionId,
  });
}

@immutable
class AgentNoteGroup {
  final String pubkey;
  final List<UserNote> notes;
  final int latestAt;
  final int earliestAt;

  const AgentNoteGroup({
    required this.pubkey,
    required this.notes,
    required this.latestAt,
    required this.earliestAt,
  });
}

List<AgentNoteGroup> groupAgentNotes(
  List<UserNote> notes, {
  int windowSeconds = 300,
}) {
  if (notes.isEmpty) return const [];

  final groups = <AgentNoteGroup>[];
  AgentNoteGroup? current;

  for (final note in notes) {
    final lastNote = current?.notes.last;
    if (current != null &&
        lastNote != null &&
        current.pubkey == note.pubkey &&
        lastNote.createdAt - note.createdAt <= windowSeconds) {
      current = AgentNoteGroup(
        pubkey: current.pubkey,
        notes: [...current.notes, note],
        latestAt: current.latestAt,
        earliestAt: note.createdAt,
      );
    } else {
      if (current != null) groups.add(current);
      current = AgentNoteGroup(
        pubkey: note.pubkey,
        notes: [note],
        latestAt: note.createdAt,
        earliestAt: note.createdAt,
      );
    }
  }

  if (current != null) groups.add(current);
  return groups;
}

@immutable
class ContactEntry {
  final String pubkey;
  final String? relayUrl;
  final String? petname;

  const ContactEntry({required this.pubkey, this.relayUrl, this.petname});

  List<String> toTag() => ['p', pubkey.toLowerCase(), ?relayUrl, ?petname];
}
