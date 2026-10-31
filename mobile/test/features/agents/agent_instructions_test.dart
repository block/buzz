import 'dart:convert';
import 'package:buzz/shared/relay/relay_author_provider.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import 'package:buzz/features/agents/agent_instructions.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:uuid/uuid.dart';

void main() {
  const channel = '89b66a68-9631-4d2f-a032-a70220b6a4aa';
  final owner = 'a' * 64;
  final agent = 'b' * 64;
  final relayAuthor = 'e' * 64;
  final artifact = const Uuid().v5(channel, 'buzz.agent.instructions');
  NostrEvent event(
    int kind,
    List<List<String>> tags,
    Object content, {
    String? author,
  }) => NostrEvent(
    id: 'c' * 64,
    pubkey: author ?? (kind == 39000 ? relayAuthor : owner),
    createdAt: 1,
    kind: kind,
    tags: tags,
    content: jsonEncode(content),
    sig: 'd' * 128,
  );
  final config = event(39000, [
    ['d', channel],
    ['P', owner],
    ['t', 'system'],
    ['t', 'agent-config'],
    ['t', agent],
    ['created_at', '1'],
  ], {});
  final legacy = event(
    30177,
    [
      ['d', agent],
    ],
    {'system_prompt': 'Legacy instructions'},
  );
  final current = event(
    45010,
    [
      ['d', artifact],
      ['h', channel],
      ['type', 'buzz.agent.instructions'],
      ['ar', '1'],
      ['op', 'update'],
    ],
    {'version': 1, 'text': 'Current instructions'},
  );

  test('relay authority uses NIP-11 self, never the contact key', () async {
    for (final info in [
      {'self': relayAuthor, 'pubkey': owner},
      {'pubkey': owner},
      {'self': 'invalid', 'pubkey': owner},
    ]) {
      final container = ProviderContainer(
        overrides: [
          relayInformationClientProvider.overrideWithValue(
            MockClient((request) async {
              expect(request.url.toString(), 'https://relay.example');
              expect(request.headers['Accept'], 'application/nostr+json');
              return http.Response(jsonEncode(info), 200);
            }),
          ),
        ],
      );
      final subscription = container.listen(
        relayAuthorProvider('wss://relay.example'),
        (_, _) {},
      );
      try {
        expect(
          await container.read(
            relayAuthorProvider('wss://relay.example').future,
          ),
          info['self'] == relayAuthor ? relayAuthor : null,
        );
      } finally {
        subscription.close();
        container.dispose();
      }
    }
  });

  test(
    'instruction sources preserve precedence and do not revive unavailable config',
    () async {
      for (final row
          in <
            ({
              List<NostrEvent> channels,
              List<NostrEvent> artifacts,
              String? expected,
            })
          >[
            (
              channels: [config],
              artifacts: [current],
              expected: 'Current instructions',
            ),
            (channels: [config], artifacts: [], expected: null),
            (
              channels: [config],
              artifacts: [
                event(45010, current.tags, {'version': 1, 'text': ''}),
              ],
              expected: '',
            ),
            (
              channels: [config],
              artifacts: [
                event(45010, [
                  ['d', artifact],
                  ['op', 'delete'],
                ], {}),
              ],
              expected: null,
            ),
            (
              channels: [
                event(39000, [
                  ...config.tags,
                  ['archived', 'true'],
                ], {}),
              ],
              artifacts: [current],
              expected: null,
            ),
            (channels: [], artifacts: [], expected: null),
            (
              channels: [event(39000, config.tags, {}, author: owner)],
              artifacts: [current],
              expected: null,
            ),
            (
              channels: [
                event(39000, [
                  ['d', channel],
                  ['P', 'f' * 64],
                  ['t', 'system'],
                  ['t', 'agent-config'],
                  ['t', agent],
                ], {}),
              ],
              artifacts: [current],
              expected: null,
            ),
          ]) {
        final result = await loadAgentInstructions(
          agent,
          owner,
          relayAuthor,
          (filter) async => switch (filter.kinds.single) {
            39000 => () {
              expect(filter.authors, [relayAuthor]);
              return row.channels;
            }(),
            45010 =>
              filter.extensions['artifact'] == 'current' &&
                      filter.tags['#h']?.single == channel
                  ? row.artifacts
                  : [],
            30177 => [legacy],
            _ => [],
          },
        );
        expect(result, row.expected);
      }
      await expectLater(
        loadAgentInstructions(
          agent,
          owner,
          relayAuthor,
          (_) async => throw StateError('offline'),
        ),
        throwsStateError,
      );
      await expectLater(
        loadAgentInstructions(
          agent,
          owner,
          relayAuthor,
          (filter) async => filter.kinds.single == 39000
              ? [config]
              : [
                  event(45010, current.tags, {'version': 2, 'text': 'Invalid'}),
                ],
        ),
        throwsFormatException,
      );
    },
  );
}
