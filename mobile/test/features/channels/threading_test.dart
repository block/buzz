import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/channels/threading.dart';
import 'package:buzz/shared/relay/relay.dart';

const _human =
    'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
const _otherHuman =
    'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd';
const _agent =
    'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';
const _otherAgent =
    'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc';

bool _isAgentPubkey(String pubkey) {
  final normalized = pubkey.trim().toLowerCase();
  return normalized == _agent || normalized == _otherAgent;
}

NostrEvent _event(
  String id,
  String pubkey, {
  List<List<String>> tags = const [],
}) {
  return NostrEvent(
    id: id,
    pubkey: pubkey,
    createdAt: 1_700_000_000,
    kind: EventKind.streamMessage,
    tags: tags,
    content: '',
    sig: 'sig',
  );
}

void main() {
  test(
    'reply to a human in an agent thread still includes the agent p-tag',
    () {
      final root = _event(
        'root1',
        _human,
        tags: [
          ['h', 'ch'],
          ['p', _agent],
        ],
      );
      final agentReply = _event(
        'reply-agent',
        _agent,
        tags: [
          ['h', 'ch'],
          ['e', 'root1', '', 'reply'],
        ],
      );
      final humanReply = _event(
        'reply-human',
        _otherHuman,
        tags: [
          ['h', 'ch'],
          ['e', 'root1', '', 'root'],
          ['e', 'reply-agent', '', 'reply'],
          ['p', _otherHuman],
        ],
      );

      final participating = collectParticipatingAgentPubkeys(
        [root, agentReply, humanReply],
        resolveReplyRootId(humanReply.id, [root, agentReply, humanReply]),
        _isAgentPubkey,
      );

      expect(participating, [_agent]);
    },
  );

  test('top-level send does not auto-tag channel agents', () {
    final topLevel = _event(
      'top1',
      _human,
      tags: [
        ['h', 'ch'],
      ],
    );
    final unrelatedAgent = _event(
      'other-agent-msg',
      _otherAgent,
      tags: [
        ['h', 'ch'],
      ],
    );
    final otherThreadRoot = _event(
      'other-root',
      _human,
      tags: [
        ['h', 'ch'],
        ['p', _agent],
      ],
    );

    final participating = collectParticipatingAgentPubkeys(
      [topLevel, unrelatedAgent, otherThreadRoot],
      topLevel.id,
      _isAgentPubkey,
    );

    expect(participating, isEmpty);
  });

  test('humans in a thread are not auto-tagged', () {
    final root = _event(
      'root1',
      _human,
      tags: [
        ['h', 'ch'],
        ['p', _agent],
        ['p', _otherHuman],
      ],
    );
    final humanReply = _event(
      'reply-human',
      _otherHuman,
      tags: [
        ['h', 'ch'],
        ['e', 'root1', '', 'reply'],
        ['p', _human],
      ],
    );

    final participating = collectParticipatingAgentPubkeys(
      [root, humanReply],
      'root1',
      _isAgentPubkey,
    );

    expect(participating, [_agent]);
    expect(participating, isNot(contains(_human)));
    expect(participating, isNot(contains(_otherHuman)));
  });
}
