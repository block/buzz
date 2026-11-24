import 'package:buzz/features/channels/timeline_message.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/thread_ids.dart';

final _author = 'a' * 64;
final _owner = 'b' * 64;
final _other = 'c' * 64;
final _relay = 'd' * 64;

NostrEvent _event({
  String id = threadIdA,
  String? author,
  int kind = EventKind.streamMessage,
  String channel = 'channel',
  String content = 'Original',
  int time = 1000,
  List<List<String>> tags = const [],
}) => NostrEvent(
  id: id,
  pubkey: author ?? _author,
  kind: kind,
  createdAt: time,
  content: content,
  tags: [
    ['h', channel],
    ...tags,
  ],
  sig: '',
);

NostrEvent _edit({
  String id = 'edit',
  String? author,
  String channel = 'channel',
  String content = 'Updated',
  int time = 2000,
  List<List<String>>? targets,
  List<List<String>> tags = const [],
}) => _event(
  id: id,
  author: author,
  kind: EventKind.streamMessageEdit,
  channel: channel,
  content: content,
  time: time,
  tags: [
    ...targets ??
        [
          ['e', threadIdA],
        ],
    ...tags,
  ],
);

void main() {
  group('edit authorship', () {
    final cases = [
      (
        name: 'the message author may edit',
        target: _event(),
        edit: _edit(author: _author.toUpperCase()),
        owners: <String, String>{},
        applied: true,
      ),
      for (final kind in [
        EventKind.streamMessage,
        EventKind.streamMessageV2,
        EventKind.streamMessageDiff,
      ])
        (
          name: 'a verified owner may edit kind $kind in the same channel',
          target: _event(kind: kind),
          edit: _edit(author: _owner),
          owners: {_author: _owner},
          applied: true,
        ),
      (
        name: 'other signers leave the message unchanged',
        target: _event(),
        edit: _edit(author: _other),
        owners: {_author: _owner},
        applied: false,
      ),
      (
        name: 'an owner edit requires the same channel',
        target: _event(),
        edit: _edit(author: _owner, channel: 'another-channel'),
        owners: {_author: _owner},
        applied: false,
      ),
      (
        name: 'an owner edit requires a verified owner entry',
        target: _event(),
        edit: _edit(author: _owner),
        owners: <String, String>{},
        applied: false,
      ),
      (
        name: 'edit owner declarations do not grant edit permission',
        target: _event(),
        edit: _edit(
          author: _owner,
          tags: [
            ['auth', _owner],
            ['owner', _owner],
          ],
        ),
        owners: <String, String>{},
        applied: false,
      ),
      (
        name: 'target owner declarations do not grant edit permission',
        target: _event(
          tags: [
            ['auth', _owner],
            ['owner', _owner],
          ],
        ),
        edit: _edit(author: _owner),
        owners: <String, String>{},
        applied: false,
      ),
      (
        name: 'a verified owner may edit a relay attributed kind 40008',
        target: _event(
          author: _relay,
          kind: EventKind.streamMessageDiff,
          tags: [
            ['actor', _author],
          ],
        ),
        edit: _edit(author: _owner),
        owners: {_author: _owner},
        applied: true,
      ),
      (
        name: 'a kind 40008 owner edit requires the same channel',
        target: _event(kind: EventKind.streamMessageDiff),
        edit: _edit(author: _owner, channel: 'another-channel'),
        owners: {_author: _owner},
        applied: false,
      ),
      (
        name: 'ordinary signers remain the author despite actor tags',
        target: _event(
          tags: [
            ['actor', _other],
          ],
        ),
        edit: _edit(author: _other),
        owners: <String, String>{},
        applied: false,
      ),
      for (final attribution in ['actor', 'p'])
        (
          name: 'a relay attributed $attribution author may edit',
          target: NostrEvent(
            id: threadIdA,
            pubkey: _relay,
            kind: EventKind.streamMessage,
            createdAt: 1000,
            content: 'Original',
            tags: [
              [attribution, _author],
              ['h', 'channel'],
            ],
            sig: '',
          ),
          edit: _edit(),
          owners: <String, String>{},
          applied: true,
        ),
      (
        name: 'a relay attributed p author may follow other tags',
        target: _event(
          author: _relay,
          tags: [
            ['p', _author.toUpperCase()],
            ['p', _other],
          ],
        ),
        edit: _edit(),
        owners: <String, String>{},
        applied: true,
      ),
      (
        name: 'the first valid p author follows invalid p tags',
        target: _event(
          author: _relay,
          tags: [
            ['p'],
            ['p', 'short'],
            ['p', 'é' * 64],
            ['p', '${'a' * 63}g'],
            ['p', _author],
            ['p', _other],
          ],
        ),
        edit: _edit(),
        owners: <String, String>{},
        applied: true,
      ),
      (
        name: 'a valid actor takes precedence over p authors',
        target: _event(
          author: _relay,
          tags: [
            ['p', _other],
            ['actor', _author],
          ],
        ),
        edit: _edit(),
        owners: <String, String>{},
        applied: true,
      ),
      (
        name: 'a verified owner may edit for a non-leading p author',
        target: _event(
          author: _relay,
          tags: [
            ['p', _author],
          ],
        ),
        edit: _edit(author: _owner),
        owners: {_author: _owner},
        applied: true,
      ),
      (
        name: 'the raw signer of an attributed message may edit',
        target: _event(
          author: _relay,
          tags: [
            ['actor', _author],
          ],
        ),
        edit: _edit(author: _relay),
        owners: <String, String>{},
        applied: true,
      ),
      (
        name: 'a verified owner of the effective author may edit',
        target: _event(
          author: _relay,
          tags: [
            ['actor', _author],
          ],
        ),
        edit: _edit(author: _owner),
        owners: {_author: _owner},
        applied: true,
      ),
    ];

    for (final scenario in cases) {
      test(scenario.name, () {
        final message = formatTimeline(
          [scenario.target, scenario.edit],
          agentOwners: scenario.owners,
          relayPubkey: _relay,
        ).single;
        expect(message.content, scenario.applied ? 'Updated' : 'Original');
        expect(message.edited, scenario.applied);
        // Every owner that can apply an edit is one the lookup asks for.
        final needs = editAuthorityNeeds([scenario.target, scenario.edit]);
        for (final MapEntry(key: author, value: owner)
            in scenario.owners.entries) {
          if (scenario.applied && scenario.edit.pubkey == owner) {
            expect(needs.owners, contains(author));
          }
        }
      });
    }
  });

  group('edit authority needs', () {
    final cases = [
      (
        name: 'a timeline without edits needs nothing',
        events: [_event()],
        owners: <String>[],
        relayKey: false,
      ),
      (
        name: 'an edit by the message signer needs nothing',
        events: [
          _event(
            tags: [
              ['actor', _other],
            ],
          ),
          _edit(),
        ],
        owners: <String>[],
        relayKey: false,
      ),
      (
        name: 'a cross-signer edit needs the author owner only',
        events: [
          _event(),
          _edit(author: _owner),
        ],
        owners: [_author],
        relayKey: false,
      ),
      (
        name: 'a kind 40008 cross-signer edit needs the author owner',
        events: [
          _event(kind: EventKind.streamMessageDiff),
          _edit(author: _owner),
        ],
        owners: [_author],
        relayKey: false,
      ),
      (
        name: 'an attributed target needs the relay key and both owners',
        events: [
          _event(
            author: _relay,
            tags: [
              ['actor', _author],
            ],
          ),
          _edit(author: _owner),
        ],
        owners: [_author, _relay],
        relayKey: true,
      ),
      (
        name: 'a p tag also justifies the relay key lookup',
        events: [
          _event(
            tags: [
              ['p', _other],
            ],
          ),
          _edit(author: _other),
        ],
        owners: [_author, _other],
        relayKey: true,
      ),
      (
        name: 'an edit from another channel needs no owner',
        events: [
          _event(),
          _edit(author: _owner, channel: 'elsewhere'),
        ],
        owners: <String>[],
        relayKey: false,
      ),
      (
        name: 'an edit whose target is outside the window needs nothing',
        events: [
          _event(),
          _edit(
            author: _owner,
            targets: [
              ['e', threadIdUnrelated],
            ],
          ),
        ],
        owners: <String>[],
        relayKey: false,
      ),
    ];
    for (final scenario in cases) {
      test(scenario.name, () {
        final needs = editAuthorityNeeds(scenario.events);
        expect(needs.owners, scenario.owners);
        expect(needs.relayKey, scenario.relayKey);
      });
    }
  });

  group('edit target selection', () {
    final cases = [
      (
        name: 'only the first valid target receives the edit',
        targets: [
          ['e', threadIdA],
          ['e', threadIdMid],
          ['e', threadIdRoot],
        ],
        author: _author,
        firstAuthor: _author,
        expected: ['Updated', 'Original', 'Original'],
      ),
      (
        name: 'authorization uses the first target only',
        targets: [
          ['e', threadIdA],
          ['e', threadIdMid],
          ['e', threadIdRoot],
        ],
        author: _author,
        firstAuthor: _other,
        expected: ['Original', 'Original', 'Original'],
      ),
      (
        name: 'malformed references are skipped before the first valid target',
        targets: [
          ['e'],
          ['e', 'short'],
          ['e', 'é' * 64],
          ['e', '${'a' * 63}g'],
          ['e', '${threadIdA}0'],
          ['e', threadIdA.toUpperCase()],
          ['e', threadIdRoot],
        ],
        author: _author,
        firstAuthor: _author,
        expected: ['Updated', 'Original', 'Original'],
      ),
      (
        name: 'a missing first target does not select a later target',
        targets: [
          ['e', threadIdUnrelated],
          ['e', threadIdRoot],
        ],
        author: _author,
        firstAuthor: _author,
        expected: ['Original', 'Original', 'Original'],
      ),
    ];
    for (final scenario in cases) {
      test(scenario.name, () {
        final messages = formatTimeline([
          _event(author: scenario.firstAuthor),
          _event(id: threadIdMid),
          _event(id: threadIdRoot),
          _edit(author: scenario.author, targets: scenario.targets),
        ]);
        expect(messages.map((message) => message.content), scenario.expected);
      });
    }
  });

  test(
    'latest selection considers only authorized edits in any input order',
    () {
      final edits = [
        _edit(id: 'earlier', content: 'Author update'),
        _edit(id: 'later', author: _other, time: 3000),
      ];
      for (final ordered in [edits, edits.reversed.toList()]) {
        final message = formatTimeline([_event(), ...ordered]).single;
        expect(message.content, 'Author update');
        expect(message.edited, isTrue);
      }
    },
  );
}
