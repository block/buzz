import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/send_message_provider.dart';
import 'package:buzz/features/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('adds DM recipients without requiring an explicit mention', () async {
    final relay = _RecordingSignedEventRelay(pubkey: 'self');
    final sendMessage = SendMessage(
      signedEventRelay: relay,
      fetchMembers: (_) async => const <ChannelMember>[],
      readUserCache: () => const <String, UserProfile>{},
    );

    await sendMessage.call(
      channelId: 'dm-channel',
      content: 'hello from mobile',
      recipientPubkeys: const ['SELF', 'AGENT', 'agent', ''],
    );

    expect(relay.tags, [
      ['h', 'dm-channel'],
      ['p', 'agent'],
    ]);
  });

  test('keeps non-DM messages mention-only', () async {
    final relay = _RecordingSignedEventRelay(pubkey: 'self');
    final sendMessage = SendMessage(
      signedEventRelay: relay,
      fetchMembers: (_) async => const <ChannelMember>[],
      readUserCache: () => const <String, UserProfile>{},
    );

    await sendMessage.call(
      channelId: 'shared-channel',
      content: 'ordinary channel message',
    );

    expect(relay.tags, [
      ['h', 'shared-channel'],
    ]);
  });

  test('deduplicates DM recipients and explicit mentions', () async {
    final relay = _RecordingSignedEventRelay(pubkey: 'self');
    final sendMessage = SendMessage(
      signedEventRelay: relay,
      fetchMembers: (_) async => const <ChannelMember>[],
      readUserCache: () => const <String, UserProfile>{},
    );

    await sendMessage.call(
      channelId: 'dm-channel',
      content: '@agent please check this',
      recipientPubkeys: const ['agent'],
      mentionPubkeys: const ['agent', 'reviewer'],
    );

    expect(relay.tags, [
      ['h', 'dm-channel'],
      ['p', 'agent'],
      ['p', 'reviewer'],
    ]);
  });
}

class _RecordingSignedEventRelay implements SignedEventRelay {
  _RecordingSignedEventRelay({required this.pubkey});

  @override
  final String? pubkey;

  List<List<String>>? tags;

  @override
  Future<NostrEvent> submit({
    required int kind,
    required String content,
    required List<List<String>> tags,
    int? createdAt,
  }) async {
    this.tags = tags;
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
