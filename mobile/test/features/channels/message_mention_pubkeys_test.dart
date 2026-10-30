import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/message_mention_pubkeys.dart';
import 'package:flutter_test/flutter_test.dart';

const _self = 'self';
const _agent = 'agent';
const _human = 'human';

void main() {
  test('implicitly addresses every participating DM recipient', () {
    expect(
      messageMentionPubkeys(
        channel: _channel(
          type: 'dm',
          participantPubkeys: const [_self, _agent, _human],
        ),
        senderPubkey: _self,
        explicitMentions: const [],
        dmRecipientPubkeys: const [_agent, _human],
      ),
      [_agent, _human],
    );
  });

  test('preserves and deduplicates explicit mentions with DM recipients', () {
    expect(
      messageMentionPubkeys(
        channel: _channel(
          type: 'dm',
          participantPubkeys: const [_self, _agent, _human],
        ),
        senderPubkey: _self,
        explicitMentions: const [_human, _agent],
        dmRecipientPubkeys: const [_agent],
      ),
      [_human, _agent],
    );
  });

  test('addresses human DMs but not ordinary channel members', () {
    expect(
      messageMentionPubkeys(
        channel: _channel(
          type: 'dm',
          participantPubkeys: const [_self, _human],
        ),
        senderPubkey: _self,
        explicitMentions: const [],
        dmRecipientPubkeys: const [_human],
      ),
      [_human],
    );
    expect(
      messageMentionPubkeys(
        channel: _channel(
          type: 'stream',
          participantPubkeys: const [_self, _agent],
        ),
        senderPubkey: _self,
        explicitMentions: const [],
        dmRecipientPubkeys: const [_agent],
      ),
      isEmpty,
    );
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
