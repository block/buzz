import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/search/search_provider.dart';
import 'package:flutter_test/flutter_test.dart';

// Channel rows rank like buzz-app search: match rank, archived half a step
// behind, joined channels first on a tie, open channels by name, top eight.
void main() {
  test('search ranks channels like desktop', () {
    final cases = <(String, List<Channel>, String, List<String>)>[
      (
        'joined wins a tie with an open channel listed and named first',
        [_channel('ops-a', isMember: false), _channel('ops-b', isMember: true)],
        'ops',
        ['ops-b', 'ops-a'],
      ),
      (
        'open channels tie by name, joined ones keep list order',
        [
          _channel('ops-z', isMember: true),
          _channel('ops-y', isMember: false),
          _channel('ops-a', isMember: true),
          _channel('ops-b', isMember: false),
        ],
        'ops',
        ['ops-z', 'ops-a', 'ops-b', 'ops-y'],
      ),
      (
        'archived ranks half a step behind and only for members',
        [
          _channel('ops-old', isMember: true, archived: true),
          _channel('ops-live', isMember: true),
          _channel('ops-gone', isMember: false, archived: true),
        ],
        'ops',
        ['ops-live', 'ops-old'],
      ),
      (
        'archived exact still leads a worse live match',
        [
          _channel('team-ops', isMember: true),
          _channel('ops', isMember: true, archived: true),
        ],
        'ops',
        ['ops', 'team-ops'],
      ),
      (
        'private channels need membership, DMs never show',
        [
          _channel('ops-private', isMember: false, visibility: 'private'),
          _channel('ops-mine', isMember: true, visibility: 'private'),
          _channel('ops-dm', isMember: true, channelType: 'dm'),
        ],
        'ops',
        ['ops-mine'],
      ),
      (
        'a leading # is ignored',
        [_channel('general', isMember: true)],
        '#gen',
        ['general'],
      ),
      (
        'a bare # lists joined channels only',
        [_channel('a', isMember: false), _channel('b', isMember: true)],
        '#',
        ['b'],
      ),
      (
        'the exact name leads past eight weaker matches, cut to eight',
        [
          for (var i = 1; i <= 9; i++) _channel('team-ops-$i', isMember: true),
          _channel('ops', isMember: false),
        ],
        'ops',
        ['ops', for (var i = 1; i <= 7; i++) 'team-ops-$i'],
      ),
    ];
    for (final (reason, channels, query, names) in cases) {
      expect(
        rankSearchChannels(channels, query).map((channel) => channel.name),
        names,
        reason: reason,
      );
    }
  });
}

Channel _channel(
  String name, {
  required bool isMember,
  bool archived = false,
  String visibility = 'open',
  String channelType = 'stream',
}) => Channel(
  id: '$name-id',
  name: name,
  channelType: channelType,
  visibility: visibility,
  description: '',
  memberCount: 3,
  createdBy: 'test',
  createdAt: DateTime(2025),
  isMember: isMember,
  archivedAt: archived ? DateTime(2025) : null,
);
