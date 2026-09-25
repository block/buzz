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
    expect(sources.missingOwnerProfiles([_human, _wesAgent]), {_wes});
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
      for (final c in candidates)
        c.withContextLabel(names.resolve(c.pubkey)?.name),
    ];
    // A query that matches only the agent still shows its contextual label.
    final ranked = rankMentionCandidates(labeled, 'hon');
    expect(ranked.map((c) => c.pickerLabel), ['Honey', 'Wes’s Honey']);
    // The inserted mention text still uses the agent's own name.
    expect(ranked.map((c) => c.label), ['Honey', 'Honey']);
  });
}
