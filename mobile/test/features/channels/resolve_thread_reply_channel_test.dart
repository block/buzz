import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/resolve_thread_reply_channel.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('returns the given channel without loading the list', () async {
    final given = _channel(id: 'general');
    var loadCalls = 0;

    final resolved = await resolveThreadReplyChannel(
      channel: given,
      channelId: 'general',
      loadChannels: () async {
        loadCalls++;
        return [given];
      },
    );

    expect(resolved, same(given));
    expect(loadCalls, 0);
  });

  test(
    'awaits the channel list and resolves by id when channel is null',
    () async {
      final wanted = _channel(id: 'general');

      final resolved = await resolveThreadReplyChannel(
        channel: null,
        channelId: 'general',
        loadChannels: () async => [_channel(id: 'random'), wanted],
      );

      expect(resolved, same(wanted));
    },
  );

  test('returns null when the loaded list has no matching channel', () async {
    final resolved = await resolveThreadReplyChannel(
      channel: null,
      channelId: 'missing',
      loadChannels: () async => [_channel(id: 'general')],
    );

    expect(resolved, isNull);
  });
}

Channel _channel({required String id}) => Channel(
  id: id,
  name: id,
  channelType: 'dm',
  visibility: 'private',
  description: '',
  createdBy: 'self',
  createdAt: DateTime(2025),
  memberCount: 1,
  participantPubkeys: const ['self'],
  isMember: true,
);
