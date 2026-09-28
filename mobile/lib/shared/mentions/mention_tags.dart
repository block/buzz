/// Mention tags on channel messages (portable mention rules, section 8).
///
/// A `["p", key]` tag addresses and notifies a recipient. A two-field
/// `["mention", key]` tag names a reference for display only. A three-field
/// `mention` tag is display metadata for an addressed agent, not a reference.
library;

/// The most keys in one list.
const maxMentionTagKeys = 32;

final _key = RegExp(r'^[0-9a-f]{64}$');

/// Whether [key] is a lowercase 64-hex public key.
bool isMentionKey(String key) => _key.hasMatch(key);

/// Why a writer refused a message's mention tags.
enum MentionTagError { notMember, invalid, tooMany }

class MentionTagException implements Exception {
  final MentionTagError error;
  const MentionTagException(this.error);

  @override
  String toString() => switch (error) {
    MentionTagError.notMember => 'A mentioned person is not in this channel',
    MentionTagError.invalid => 'A mention has an invalid key',
    MentionTagError.tooMany => 'A message can mention at most 32 people',
  };
}

/// The ordered `p` and `mention` tags for one message. [members] must hold
/// the channel members, including the sender. Throws [MentionTagException]
/// instead of dropping or converting a bad key.
List<List<String>> writeMentionTags({
  required Iterable<String> recipients,
  required Iterable<String> references,
  required Set<String> members,
}) {
  List<String> unique(Iterable<String> keys) {
    final list = <String>[];
    for (final key in keys) {
      if (!isMentionKey(key)) {
        throw const MentionTagException(MentionTagError.invalid);
      }
      if (!list.contains(key)) list.add(key);
    }
    if (list.length > maxMentionTagKeys) {
      throw const MentionTagException(MentionTagError.tooMany);
    }
    return list;
  }

  final p = unique(recipients);
  final mention = unique(references);
  if (p.any((key) => !members.contains(key))) {
    throw const MentionTagException(MentionTagError.notMember);
  }
  return [
    for (final key in p) ['p', key],
    for (final key in mention) ['mention', key],
  ];
}

/// The keys a message addresses ([mentions]) and names without notifying
/// ([references]), in first-seen order. Invalid keys are ignored. A key in
/// both lists is addressed.
({List<String> mentions, List<String> references}) readMentionTags(
  Iterable<List<String>> tags,
) {
  final mentions = <String>[];
  final references = <String>[];
  for (final tag in tags) {
    if (tag.length < 2 || !isMentionKey(tag[1])) continue;
    if (tag[0] == 'p') {
      if (!mentions.contains(tag[1])) mentions.add(tag[1]);
    } else if (tag[0] == 'mention' && tag.length == 2) {
      if (!references.contains(tag[1])) references.add(tag[1]);
    }
  }
  return (mentions: mentions, references: references);
}

/// Every key a message names, for resolving profile names.
Set<String> mentionedPubkeysFromTags(Iterable<List<String>> tags) {
  final read = readMentionTags(tags);
  return {...read.mentions, ...read.references};
}
