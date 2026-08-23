import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/message_recipients.dart';
import 'package:flutter_test/flutter_test.dart';

const _self = 'self';
const _agent = 'agent';
const _human = 'human';

void main() {
  test('implicitly addresses every participating DM recipient', () {
    final recipients = messageRecipients(
      channel: _channel(
        type: 'dm',
        participantPubkeys: const [_self, _agent, _human],
      ),
      senderPubkey: _self,
      explicitMentions: const [],
      dmRecipientPubkeys: const [_agent, _human],
    );
    // Addressed by the channel, not mentioned: nobody typed their names, so
    // marking these as mentions would pierce a mute and outrank a real `@you`.
    expect(recipients.mentions, isEmpty);
    expect(recipients.addressed, [_agent, _human]);
  });

  test('preserves and deduplicates explicit mentions with DM recipients', () {
    final recipients = messageRecipients(
      channel: _channel(
        type: 'dm',
        participantPubkeys: const [_self, _agent, _human],
      ),
      senderPubkey: _self,
      explicitMentions: const [_human, _agent],
      dmRecipientPubkeys: const [_agent],
    );
    // Typed wins: a participant who was also written as `@name` is a mention,
    // and is not repeated as a channel recipient.
    expect(recipients.mentions, [_human, _agent]);
    expect(recipients.addressed, isEmpty);
  });

  test('addresses human DMs but not ordinary channel members', () {
    final dm = messageRecipients(
      channel: _channel(type: 'dm', participantPubkeys: const [_self, _human]),
      senderPubkey: _self,
      explicitMentions: const [],
      dmRecipientPubkeys: const [_human],
    );
    expect(dm.addressed, [_human]);

    final stream = messageRecipients(
      channel: _channel(
        type: 'stream',
        participantPubkeys: const [_self, _agent],
      ),
      senderPubkey: _self,
      explicitMentions: const [],
      dmRecipientPubkeys: const [_agent],
    );
    expect(stream.mentions, isEmpty);
    expect(stream.addressed, isEmpty);
  });

  test('never addresses the sender', () {
    final recipients = messageRecipients(
      channel: _channel(type: 'dm', participantPubkeys: const [_self, _human]),
      senderPubkey: _self,
      explicitMentions: const [_self],
      dmRecipientPubkeys: const [_self, _human],
    );
    expect(recipients.mentions, isEmpty);
    expect(recipients.addressed, [_human]);
  });
}

Channel _channel({
  required String type,
  required List<String> participantPubkeys,
}) => Channel(
  id: 'channel',
  name: 'Conversation',
  channelType: type,
  visibility: 'private',
  description: '',
  createdBy: _self,
  createdAt: DateTime(2025),
  memberCount: participantPubkeys.length,
  participantPubkeys: participantPubkeys,
  isMember: true,
);
