import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/channels/send_message_provider.dart';
import 'package:buzz/features/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';

void main() {
  test(
    'plain DM appends the channel participant without a picker mention',
    () async {
      final tags = await _send(dmParticipantPubkeys: ['Recipient']);

      expect(tags, [
        ['h', 'dm-channel'],
        ['p', 'Recipient'],
      ]);
    },
  );

  test(
    'DM mention tags are deduplicated and self-excluded case-insensitively',
    () async {
      final tags = await _send(
        mentionPubkeys: ['EXPLICIT', 'Peer', 'sender'],
        dmParticipantPubkeys: ['PEER', 'SENDER', 'second'],
      );

      expect(tags, [
        ['h', 'dm-channel'],
        ['p', 'EXPLICIT'],
        ['p', 'Peer'],
        ['p', 'second'],
      ]);
    },
  );

  test('non-DM keeps explicit mention semantics', () async {
    final tags = await _send(
      mentionPubkeys: ['Explicit'],
      dmParticipantPubkeys: null,
    );

    expect(tags, [
      ['h', 'dm-channel'],
      ['p', 'Explicit'],
    ]);
  });

  test('threaded DM keeps e-tags before p-tags and media tags last', () async {
    final tags = await _send(
      mentionPubkeys: ['Explicit'],
      dmParticipantPubkeys: ['Recipient'],
      parentEventId: 'parent',
      rootEventId: 'root',
      mediaTags: const [
        ['imeta', 'blob'],
        ['emoji', 'party'],
      ],
    );

    expect(tags, [
      ['h', 'dm-channel'],
      ['e', 'root', '', 'root'],
      ['e', 'parent', '', 'reply'],
      ['p', 'Explicit'],
      ['p', 'Recipient'],
      ['imeta', 'blob'],
      ['emoji', 'party'],
    ]);
  });
}

Future<List<List<String>>> _send({
  List<String> mentionPubkeys = const [],
  List<String>? dmParticipantPubkeys,
  String? parentEventId,
  String? rootEventId,
  List<List<String>> mediaTags = const [],
}) async {
  final relay = _RecordingSignedEventRelay();
  final sender = SendMessage(
    signedEventRelay: relay,
    fetchMembers: (_) async => const [],
    readUserCache: () => const <String, UserProfile>{},
  );

  await sender(
    channelId: 'dm-channel',
    content: 'message',
    mentionPubkeys: mentionPubkeys,
    dmParticipantPubkeys: dmParticipantPubkeys,
    parentEventId: parentEventId,
    rootEventId: rootEventId,
    mediaTags: mediaTags,
  );

  expect(relay.submittedKind, EventKind.streamMessage);
  return relay.submittedTags!;
}

class _RecordingSignedEventRelay implements SignedEventRelay {
  @override
  String? get pubkey => 'SENDER';

  int? submittedKind;
  List<List<String>>? submittedTags;

  @override
  Future<NostrEvent> submit({
    required int kind,
    required String content,
    required List<List<String>> tags,
    int? createdAt,
  }) async {
    submittedKind = kind;
    submittedTags = tags;
    return const NostrEvent(
      id: 'stub',
      pubkey: '',
      createdAt: 0,
      kind: 0,
      tags: [],
      content: '',
      sig: '',
    );
  }
}
