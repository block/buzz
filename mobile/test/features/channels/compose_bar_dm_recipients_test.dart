import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/compose_bar.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('DM messages notify every participant except the sender', () {
    final recipients = messageNotificationPubkeys(
      explicitMentions: const ['agent'],
      channelMembers: [_member('owner'), _member('agent'), _member('observer')],
      currentPubkey: 'OWNER',
      isDirectMessage: true,
    );

    expect(recipients, ['agent', 'observer']);
  });

  test('regular channel messages only notify explicit mentions', () {
    final recipients = messageNotificationPubkeys(
      explicitMentions: const ['agent'],
      channelMembers: [_member('owner'), _member('observer')],
      currentPubkey: 'owner',
      isDirectMessage: false,
    );

    expect(recipients, ['agent']);
  });
}

ChannelMember _member(String pubkey) => ChannelMember(
  pubkey: pubkey,
  role: 'member',
  joinedAt: DateTime.fromMillisecondsSinceEpoch(0),
);
