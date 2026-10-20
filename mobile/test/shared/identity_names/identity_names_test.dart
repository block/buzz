import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/mentions/mention_candidates.dart';
import 'package:buzz/features/channels/mentions/mention_candidates_provider.dart';
import 'package:buzz/features/channels/mentions/mention_ranking.dart';
import 'package:buzz/shared/identity_names/identity_names.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/utils/string_utils.dart';
import 'package:flutter_test/flutter_test.dart';

String _key(String digit) => digit * 64;

final _logan = _key('1');
final _wes = _key('2');
final _human = _key('c');
final _mine = _key('a');
final _wesAgent = _key('b');

IdentityNameSources _sources() => IdentityNameSources(
  viewer: _logan,
  profiles: {
    _logan: UserProfile(pubkey: _logan, displayName: 'Logan'),
    _wes: UserProfile(pubkey: _wes, displayName: 'Wes'),
    _human: UserProfile(pubkey: _human, displayName: 'Honey'),
    _mine: UserProfile(
      pubkey: _mine,
      displayName: 'Honey',
      ownerPubkey: _logan,
    ),
    _wesAgent: UserProfile(
      pubkey: _wesAgent,
      displayName: 'Honey',
      ownerPubkey: _wes,
    ),
  },
);

void main() {
  for (final source in ['directory', 'channel role', 'profile owner']) {
    test('agent suffixes from $source distinguish all identity labels', () {
      final keys = [_mine, _wesAgent];
      final names = IdentityNameSources(
        agentPubkeys: source == 'directory' ? keys.toSet() : {},
        profiles: {
          for (final key in keys)
            key: UserProfile(
              pubkey: key,
              displayName: 'Scout',
              ownerPubkey: source == 'profile owner' ? _wes : null,
            ),
        },
      ).scope(keys, agentPubkeys: source == 'channel role' ? keys.toSet() : {});
      for (final key in keys) {
        expect(names.labelFor(key.toUpperCase()), names.resolve(key)!.name);
        final resolved = names.resolve(key)!;
        expect(resolved.qualifier, isNotNull);
        expect(names.labelFor(key), 'Scout · ${resolved.qualifier}');
      }
      expect(names.resolve(_mine)!.name, isNot(names.resolve(_wesAgent)!.name));
    });
  }

  test('agent display keeps readable owners and literal name punctuation', () {
    final names = IdentityNameSources(
      viewer: _logan,
      profiles: {
        _wes: UserProfile(pubkey: _wes, displayName: 'Wes'),
        for (final key in [_mine, _wesAgent])
          key: UserProfile(
            pubkey: key,
            displayName: 'Scout · 1234',
            ownerPubkey: _wes,
          ),
      },
    ).scope([_mine, _wesAgent]);
    for (final key in [_mine, _wesAgent]) {
      expect(names.labelFor(key), startsWith('Wes’s Scout · 1234 · '));
      expect(names.resolve(key)!.qualifier, isNotNull);
    }
    expect(
      IdentityNameSources(
        agentDisplayNames: {_mine: 'Scout · 1234'},
        agentPubkeys: {_mine},
      ).scope([_mine]).labelFor(_mine),
      'Scout · 1234',
    );
  });

  test('human identity suffixes remain visible', () {
    final names = IdentityNameSources(
      profiles: {
        for (final key in [_mine, _wesAgent])
          key: UserProfile(pubkey: key, displayName: 'Scout'),
      },
    ).scope([_mine, _wesAgent]);
    for (final key in [_mine, _wesAgent]) {
      expect(names.resolve(key)!.qualifier, isNotNull);
      expect(names.labelFor(key), names.resolve(key)!.name);
    }
  });

  test('channel context: human, then mine, then owner-qualified others', () {
    // Wes is not a member: his profile is only an owner lookup fact.
    final names = _sources().scope([_logan, _human, _mine, _wesAgent]);
    expect(names.labelFor(_human), 'Honey');
    expect(names.labelFor(_mine), 'Honey (agent)');
    expect(names.labelFor(_wesAgent), 'Wes’s Honey');
    expect(names.labelFor(_logan), 'Logan');
  });

  test('a non-member reference is compared with the members plus itself', () {
    final names = _sources().scope([_human]);
    // Alone, Logan's agent is plain; against the human it is marked.
    expect(_sources().scope([_mine]).labelFor(_mine), 'Honey');
    expect(names.labelFor(_mine), 'Honey (agent)');
    expect(names.labelFor(_human), 'Honey');
  });

  test('profiles outside the context do not create collisions', () {
    final names = _sources().scope([_wesAgent]);
    expect(names.labelFor(_wesAgent), 'Honey');
  });

  test('view-local agent roles and fallback names become facts', () {
    final bot = _key('d');
    final names = IdentityNameSources(
      profiles: {_human: UserProfile(pubkey: _human, displayName: 'Honey')},
    ).scope([_human, bot], agentPubkeys: {bot}, fallbackNames: {bot: 'Honey'});
    expect(names.labelFor(bot), 'Honey (agent)');
    expect(names.labelFor(_human), 'Honey');
  });

  test(
    'blank names fall back to the compact npub; invalid keys stay plain',
    () {
      final names = IdentityNameSources(
        profiles: {
          _human: UserProfile(pubkey: _human, displayName: '  '),
          'legacy': const UserProfile(pubkey: 'legacy', displayName: 'Honey'),
        },
      ).scope([_human, 'not-a-key', 'legacy']);
      expect(names.candidates, {_human});
      expect(names.labelFor(_human), shortPubkey(_human));
      expect(names.labelFor('not-a-key'), unknownIdentityLabel);
      // A malformed key is not an identity fact: plain name, no qualifier.
      expect(names.labelFor('legacy'), 'Honey');
    },
  );

  test('reports owner profiles to load, without inventing owner names', () {
    final sources = IdentityNameSources(
      viewer: _logan,
      profiles: {
        _human: UserProfile(pubkey: _human, displayName: 'Honey'),
        _wesAgent: UserProfile(
          pubkey: _wesAgent,
          displayName: 'Honey',
          ownerPubkey: _wes,
        ),
      },
    );
    expect(sources.scope([_human, _wesAgent]).missingOwnerProfiles(), {_wes});
    expect(
      sources.scope([_human, _wesAgent]).labelFor(_wesAgent),
      'Honey (agent)',
    );
  });

  test('mention picker labels compare all choices but keep wire names', () {
    final candidates = [
      MentionCandidate(pubkey: _human, displayName: 'Honey', isMember: true),
      MentionCandidate(
        pubkey: _wesAgent,
        displayName: 'Honey',
        isAgent: true,
        ownerPubkey: _wes,
      ),
    ];
    final names = mentionPickerNames(_sources(), candidates);
    final labeled = [
      for (final c in candidates) c.withContextLabel(names.labelFor(c.pubkey)),
    ];
    // A query that matches only the agent still shows its contextual label.
    final ranked = rankMentionCandidates(labeled, 'hon');
    expect(ranked.map((c) => c.pickerLabel), ['Honey', 'Wes’s Honey']);
    // The inserted mention text still uses the agent's own name.
    expect(ranked.map((c) => c.label), ['Honey', 'Honey']);
  });

  test('a searched agent keeps its owner before its profile is cached', () {
    final scout = _key('d');
    final myScout = _key('e');
    // Only the viewer's profile is cached; the viewer's newly searched agent
    // is not, and the member bot has no known owner.
    final sources = IdentityNameSources(
      viewer: _logan,
      profiles: {_logan: UserProfile(pubkey: _logan, displayName: 'Logan')},
    );
    final candidates = buildMentionCandidates(
      members: [
        ChannelMember(
          pubkey: scout,
          role: 'bot',
          joinedAt: DateTime(2026),
          displayName: 'Scout',
        ),
      ],
      relayAgents: const [],
      sharedChannelIds: const {},
      userCache: sources.profiles,
      ownerByAgentPubkey: const {},
      searchResults: [
        UserProfile(pubkey: myScout, displayName: 'Scout', ownerPubkey: _logan),
      ],
      currentPubkey: _logan,
    );
    expect(candidates.map((c) => c.pubkey), [scout, myScout]);
    final names = mentionPickerNames(sources, candidates);
    expect(names.labelFor(myScout), 'Scout');
    expect(names.labelFor(scout), isNot(names.labelFor(myScout)));
    expect(names.labelFor(scout), isNot('Scout'));
    // The owner is the viewer, whose profile is cached: nothing to load.
    expect(names.missingOwnerProfiles(), isEmpty);
  });
}
