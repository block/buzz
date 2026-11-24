import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/dm_channel_labels.dart';
import 'package:buzz/shared/identity_names/identity_names.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/utils/string_utils.dart';
import 'package:flutter_test/flutter_test.dart';

String _key(int number) => number.toRadixString(16).padLeft(64, '0');

Channel _dm(
  String id, {
  String name = 'DM',
  List<String> keys = const [],
  List<String> labels = const [],
}) => Channel(
  id: id,
  name: name,
  channelType: 'dm',
  visibility: 'private',
  description: '',
  createdBy: _key(1),
  createdAt: DateTime(2026),
  memberCount: keys.length,
  participantPubkeys: keys,
  participants: labels,
  isMember: true,
);

// Count real contextual resolutions, without replacing the naming policy.
class _CountingSources extends IdentityNameSources {
  final List<List<String>> scopes = [];

  _CountingSources({required super.profiles});

  @override
  IdentityNames scope(
    Iterable<String> candidates, {
    Set<String> agentPubkeys = const {},
    Map<String, String> fallbackNames = const {},
    Map<String, String> ownerPubkeys = const {},
  }) {
    scopes.add(candidates.toList());
    return super.scope(
      candidates,
      agentPubkeys: agentPubkeys,
      fallbackNames: fallbackNames,
      ownerPubkeys: ownerPubkeys,
    );
  }
}

void main() {
  test('sorting resolves each DM context once, independent of comparisons', () {
    final channels = [
      for (var index = 0; index < 80; index++)
        _dm('dm-$index', keys: [_key(1), _key(index + 2)]),
    ];
    final names = _CountingSources(
      profiles: {
        for (var index = 0; index < channels.length; index++)
          _key(index + 2): UserProfile(
            pubkey: _key(index + 2),
            displayName: 'Person ${(79 - index).toString().padLeft(2, '0')}',
          ),
      },
    );

    final sorted = sortDmChannelsByDisplayLabel(
      channels,
      currentPubkey: _key(1),
      names: names,
    );

    expect(sorted, channels.reversed.toList());
    expect(names.scopes, hasLength(channels.length));
    expect(
      names.scopes,
      unorderedEquals(channels.map((channel) => channel.participantPubkeys)),
    );
    expect(channels.first.id, 'dm-0', reason: 'Do not mutate the source list');
  });

  test(
    'each sort uses current labels, identity, participants and community',
    () {
      final me = _key(1);
      final alice = _key(2);
      final agent = _key(3);
      final otherAgent = _key(4);
      final owner = _key(5);
      final channels = [
        _dm('direct', keys: [me, alice], labels: ['Zoe', 'Alice']),
        _dm(
          'group',
          name: 'Group DM (4)',
          keys: [me, alice, agent, otherAgent],
          labels: ['Zoe', 'Alice', 'Scout', 'Scout'],
        ),
        _dm('self', keys: [me], labels: ['Zoe']),
        _dm('empty'),
        _dm('legacy', keys: ['legacy-key'], labels: ['Bob']),
        _dm('tie-b', name: 'ALICE'),
        _dm('tie-a', name: 'alice'),
      ];
      final updatedNames = IdentityNameSources(
        viewer: me,
        profiles: {
          me: UserProfile(pubkey: me, displayName: 'Zoe'),
          alice: UserProfile(pubkey: alice, displayName: 'Yvonne'),
          agent: UserProfile(
            pubkey: agent,
            displayName: 'Scout',
            ownerPubkey: me,
          ),
          otherAgent: UserProfile(
            pubkey: otherAgent,
            displayName: 'Scout',
            ownerPubkey: owner,
          ),
          owner: UserProfile(pubkey: owner, displayName: 'Aaron'),
        },
      );
      final cases = [
        (channels: channels, viewer: me, names: null),
        (channels: channels, viewer: me, names: updatedNames),
        (channels: channels, viewer: alice, names: updatedNames),
        (
          channels: [
            channels.first.copyWith(name: 'Renamed'),
            _dm('group', keys: [agent, owner], labels: ['Scout', 'Aaron']),
            ...channels.skip(2),
          ],
          viewer: me,
          names: updatedNames,
        ),
        (channels: channels, viewer: me, names: const IdentityNameSources()),
        (channels: <Channel>[], viewer: me, names: updatedNames),
        (channels: [channels.first], viewer: me, names: updatedNames),
      ];

      for (final scenario in cases) {
        // Independent reference: the previous comparator's ordering contract.
        final expected = scenario.channels.toList()
          ..sort((left, right) {
            String label(Channel channel) => resolveDmChannelDisplayLabel(
              channel,
              currentPubkey: scenario.viewer,
              names: scenario.names,
            ).toLowerCase();
            final order = label(left).compareTo(label(right));
            return order != 0 ? order : left.id.compareTo(right.id);
          });
        expect(
          sortDmChannelsByDisplayLabel(
            scenario.channels,
            currentPubkey: scenario.viewer,
            names: scenario.names,
          ),
          expected,
        );
      }
      expect(
        sortDmChannelsByDisplayLabel(
          channels,
          currentPubkey: me,
        ).map((channel) => channel.id),
        ['direct', 'tie-a', 'tie-b', 'group', 'legacy', 'empty', 'self'],
      );
    },
  );

  test('a DM is named by its people, never its agents, unless it has none', () {
    final me = _key(1);
    final ana = _key(2), ben = _key(3), cy = _key(4), dee = _key(5);
    final bot = _key(6), helper = _key(7), unloaded = _key(8);
    final names = IdentityNameSources(
      profiles: {
        for (final (key, name) in [
          (ana, 'Ana'),
          (ben, 'Ben'),
          (cy, 'Cy'),
          (dee, 'Dee'),
        ])
          key: UserProfile(pubkey: key, displayName: name),
        bot: UserProfile(pubkey: bot, displayName: 'Bot', ownerPubkey: me),
        helper: UserProfile(
          pubkey: helper,
          displayName: 'Helper',
          ownerPubkey: me,
        ),
      },
    );
    String label(List<String> keys) => resolveDmChannelDisplayLabel(
      _dm('dm', keys: [me, ...keys]),
      currentPubkey: me,
      names: names,
    );

    // People and agents: only the people.
    expect(label([bot, ana]), 'Ana');
    // Only agents: no other person, so the agents.
    expect(label([bot, helper]), 'Bot, Helper');
    // More than three people and agents: "+N more" counts people only.
    expect(label([bot, ana, ben, helper, cy, dee]), 'Ana, Ben, Cy, +1 more');
    // A profile not loaded yet is named until it says it is an agent.
    expect(label([bot, unloaded]), shortPubkey(unloaded));
  });
}
