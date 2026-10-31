import 'dart:convert';

import 'package:buzz/features/agents/agent_instructions.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:uuid/uuid.dart';

void main() {
  const channel = '89b66a68-9631-4d2f-a032-a70220b6a4aa';
  final owner = 'a' * 64;
  final agent = 'b' * 64;
  final artifact = const Uuid().v5(channel, 'buzz.agent.instructions');
  NostrEvent event(
    int kind,
    List<List<String>> tags,
    Object content, {
    String? author,
  }) => NostrEvent(
    id: 'c' * 64,
    pubkey: author ?? owner,
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
            (channels: [], artifacts: [], expected: 'Legacy instructions'),
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
              expected: 'Legacy instructions',
            ),
          ]) {
        final result = await loadAgentInstructions(
          agent,
          owner,
          (filter) async => switch (filter.kinds.single) {
            39000 => row.channels,
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
      // Coordinates match desktop persona_events::normalize_d_tag, including
      // built-in IDs and the mixed-case/leading punctuation of team-pack slugs.
      for (final (personaId, slug) in [
        ('helper', 'helper'),
        ('builtin:fizz', 'builtin-fizz'),
        ('builtin:honey', 'builtin-honey'),
        ('builtin:bumble', 'builtin-bumble'),
        ('CodeReviewer', 'codereviewer'),
        ('_Ops', 'a_ops'),
        ('-Ops', 'a-ops'),
        ('É😀Ops', 'a--ops'),
        ('', 'a'),
        ('A' * 65, 'a' * 64),
      ]) {
        final linked = event(
          30177,
          [
            ['d', agent],
          ],
          {'system_prompt': 'Stale', 'persona_id': personaId},
        );
        for (final available in [true, false]) {
          expect(
            await loadAgentInstructions(
              agent,
              owner,
              (filter) async => switch (filter.kinds.single) {
                30177 => [linked],
                30175
                    when available &&
                        filter.authors?.single == owner &&
                        filter.tags['#d']?.single == slug =>
                  [
                    event(
                      30175,
                      [
                        ['d', slug],
                      ],
                      {'system_prompt': 'Definition instructions'},
                    ),
                  ],
                _ => [],
              },
            ),
            available ? 'Definition instructions' : null,
            reason: 'persona_id: $personaId, definition available: $available',
          );
        }
      }
      await expectLater(
        loadAgentInstructions(
          agent,
          owner,
          (_) async => throw StateError('offline'),
        ),
        throwsStateError,
      );
      await expectLater(
        loadAgentInstructions(
          agent,
          owner,
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
