import 'package:buzz/features/channels/agent_activity_runs.dart';
import 'package:buzz/features/channels/timeline_message.dart';
import 'package:flutter_test/flutter_test.dart';

const _quiet = ['audience', 'agents'];

TimelineMessage _row(
  String id, {
  String pubkey = 'agent',
  bool quiet = true,
  int createdAt = 1000,
  String? parentId,
  String? rootId,
  String? request,
  bool system = false,
}) => TimelineMessage(
  id: id,
  pubkey: pubkey,
  createdAt: createdAt,
  content: id,
  tags: [
    if (quiet) _quiet,
    if (request != null) ['buzz:request', request],
  ],
  isSystem: system,
  parentId: parentId,
  rootId: rootId,
);

TimelineMessage _human(String id, {String? rootId}) =>
    _row(id, pubkey: 'human', quiet: false, rootId: rootId, parentId: rootId);

/// Each run as its message IDs, in order.
List<List<String>> _runs(
  List<TimelineMessage> rows, {
  List<TimelineMessage> loaded = const [],
  String? rootId,
}) {
  final runs = activityRuns(rows, loaded: loaded, rootId: rootId);
  return [
    for (final run in {...runs.values}) [for (final row in run.rows) row.id],
  ];
}

void main() {
  final day = DateTime(2026, 11, 12, 12).millisecondsSinceEpoch ~/ 1000;
  final cases = <String, (List<TimelineMessage>, List<List<String>>)>{
    'a single step is a run': (
      [_human('q'), _row('a')],
      [
        ['a'],
      ],
    ),
    'different agents share a run': (
      [_row('a'), _row('b', pubkey: 'other'), _row('c')],
      [
        ['a', 'b', 'c'],
      ],
    ),
    'a human message splits runs': (
      [_row('a'), _human('q'), _row('b')],
      [
        ['a'],
        ['b'],
      ],
    ),
    'a public agent reply splits runs': (
      [_row('a'), _row('reply', quiet: false), _row('b')],
      [
        ['a'],
        ['b'],
      ],
    ),
    'a system notice splits runs and never joins one': (
      [_row('a'), _row('joined', system: true), _row('b')],
      [
        ['a'],
        ['b'],
      ],
    ),
    'a new local day splits runs': (
      [_row('a', createdAt: day), _row('b', createdAt: day + 86400)],
      [
        ['a'],
        ['b'],
      ],
    ),
    'a new conversation splits runs': (
      [_row('a'), _row('b', rootId: 'thread')],
      [
        ['a'],
        ['b'],
      ],
    ),
    'claims of different requests split runs': (
      [
        _human('q1'),
        _human('q2'),
        _row('a', request: 'q1'),
        _row('b', request: 'q2'),
        _row('c', request: 'q2'),
      ],
      [
        ['a'],
        ['b', 'c'],
      ],
    ),
    'a claim naming no loaded human request falls back': (
      [_human('q'), _row('a'), _row('b', request: 'missing')],
      [
        ['a', 'b'],
      ],
    ),
    'a claim naming an agent message falls back': (
      [_human('q'), _row('a'), _row('b', request: 'a')],
      [
        ['a', 'b'],
      ],
    ),
    'a reply parent names its request': (
      [_human('q1'), _human('q2'), _row('a', parentId: 'q1'), _row('b')],
      [
        ['a'],
        ['b'],
      ],
    ),
    'a reply to a step inherits its request': (
      [
        _human('q1'),
        _human('q2'),
        _row('a', parentId: 'q1'),
        _row('b', parentId: 'a'),
      ],
      [
        ['a', 'b'],
      ],
    ),
  };
  for (final MapEntry(key: name, value: (rows, expected)) in cases.entries) {
    test(name, () => expect(_runs(rows), expected));
  }

  test('a claim may name a request loaded at another level', () {
    final elsewhere = _human('q1');
    expect(
      _runs(
        [_human('q2'), _row('a', request: 'q1'), _row('b')],
        loaded: [elsewhere],
      ),
      [
        ['a'],
        ['b'],
      ],
    );
  });

  test('in a thread, a claim of the root and replies to it share the root', () {
    final replies = [
      _row('a', rootId: 'root', parentId: 'root', request: 'root'),
      _row('b', rootId: 'root', parentId: 'root'),
      _human('later', rootId: 'root'),
      _row('c', rootId: 'root', parentId: 'root'),
      _row('d', rootId: 'root', parentId: 'later'),
    ];
    expect(_runs(replies, loaded: [_human('root')], rootId: 'root'), [
      ['a', 'b'],
      ['c'],
      ['d'],
    ]);
  });

  test('every message of a run maps to the same run', () {
    final rows = [_row('a'), _row('b')];
    final runs = activityRuns(rows);
    expect(identical(runs['a'], runs['b']), isTrue);
    expect(runs['a']!.participants, ['agent']);
    expect(runs['a']!.latest.id, 'b');
  });
}
