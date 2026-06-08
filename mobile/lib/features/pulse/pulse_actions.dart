import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import '../channels/channel_management_provider.dart';
import '../custom_emoji/custom_emoji.dart';
import '../custom_emoji/custom_emoji_provider.dart';
import 'pulse_models.dart';
import 'pulse_provider.dart';

Future<void> publishNote(
  WidgetRef ref, {
  required String content,
  UserNote? replyTo,
  List<String> mentionPubkeys = const [],
  List<List<String>> mediaTags = const [],
}) async {
  final text = content.trim();
  if (text.isEmpty) return;

  final config = ref.read(relayConfigProvider);
  final session = ref.read(relaySessionProvider.notifier);
  final relay = SignedEventRelay(session: session, nsec: config.nsec);
  final tags = <List<String>>[];
  final seen = <String>{};

  if (replyTo != null) {
    tags.add(['e', replyTo.id, '', 'reply']);
    seen.add(replyTo.pubkey.toLowerCase());
    tags.add(['p', replyTo.pubkey.toLowerCase()]);
  }

  for (final pubkey in mentionPubkeys) {
    final normalized = pubkey.toLowerCase();
    if (seen.add(normalized)) tags.add(['p', normalized]);
  }
  tags.addAll(mediaTags);
  tags.addAll(buildCustomEmojiTags(text, ref.read(customEmojiListProvider)));

  await relay.submit(kind: EventKind.note, content: text, tags: tags);
  ref.invalidate(globalNotesProvider);
  ref.invalidate(likedNotesProvider);
}

Future<void> toggleNoteUpvote(
  WidgetRef ref, {
  required String noteId,
  required bool isUpvoted,
  String? reactionEventId,
}) async {
  final actions = ref.read(channelActionsProvider);
  if (isUpvoted) {
    if (reactionEventId != null) {
      await actions.removeReaction(reactionEventId, '+');
    }
  } else {
    await actions.addReaction(noteId, '+');
  }
  ref.invalidate(likedNotesProvider);
}

Future<void> setContactList(WidgetRef ref, List<ContactEntry> contacts) async {
  final currentPubkey = ref.read(myPubkeyProvider);
  if (currentPubkey == null) return;
  final config = ref.read(relayConfigProvider);
  final session = ref.read(relaySessionProvider.notifier);
  final relay = SignedEventRelay(session: session, nsec: config.nsec);

  await relay.submit(
    kind: EventKind.contactList,
    content: '',
    tags: contacts.map((entry) => entry.toTag()).toList(),
  );
  ref.invalidate(contactListProvider(currentPubkey));
}

Future<void> followUser(WidgetRef ref, String pubkey) async {
  final currentPubkey = ref.read(myPubkeyProvider);
  if (currentPubkey == null) return;
  final contacts = await _fetchFreshContacts(ref, currentPubkey);
  final normalized = pubkey.toLowerCase();
  if (contacts.any((entry) => entry.pubkey == normalized)) return;
  await setContactList(ref, [...contacts, ContactEntry(pubkey: normalized)]);
}

Future<void> unfollowUser(WidgetRef ref, String pubkey) async {
  final currentPubkey = ref.read(myPubkeyProvider);
  if (currentPubkey == null) return;
  final normalized = pubkey.toLowerCase();
  final contacts = await _fetchFreshContacts(ref, currentPubkey);
  await setContactList(
    ref,
    contacts.where((entry) => entry.pubkey != normalized).toList(),
  );
}

Future<List<ContactEntry>> _fetchFreshContacts(
  WidgetRef ref,
  String currentPubkey,
) async {
  final session = ref.read(relaySessionProvider.notifier);
  final events = await session.fetchHistory(
    NostrFilters.contactList(currentPubkey),
  );
  return contactsFromEvents(events);
}
