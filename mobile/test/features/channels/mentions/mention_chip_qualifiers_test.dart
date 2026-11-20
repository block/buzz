import 'dart:math';

import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/compose_bar.dart';
import 'package:buzz/features/channels/mentions/mention_candidates.dart';
import 'package:buzz/features/channels/mentions/mention_candidates_provider.dart';
import 'package:buzz/features/channels/mentions/mention_ranking.dart';
import 'package:buzz/shared/identity_names/identity_names.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/utils/string_utils.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';

/// Chip labels as they were computed before they were scoped: every member
/// and every chip named together.
Map<String, String> _amongTheWholeRoster(
  IdentityNameSources sources,
  List<ChannelMember> members,
  Map<String, String> agentOwners,
  List<MentionCandidate> chips,
) {
  final names = mentionPickerNames(sources, [
    ...buildMentionCandidates(
      members: members,
      relayAgents: const [],
      sharedChannelIds: const {},
      userCache: sources.profiles,
      ownerByAgentPubkey: agentOwners,
    ),
    ...chips,
  ]);
  return {
    for (final chip in chips)
      if (names.resolve(chip.pubkey)?.qualifier case final suffix?)
        chip.pubkey.toLowerCase(): '${chip.label} · $suffix',
  };
}

void main() {
  String key(int i) => (i + 1).toRadixString(16).padLeft(64, '0');
  ChannelMember member(int i, String? name, {bool bot = false}) =>
      ChannelMember(
        pubkey: key(i),
        displayName: name,
        role: bot ? 'bot' : 'member',
        joinedAt: DateTime(2025),
      );

  test('chips are labelled among their namesakes as among the whole roster, '
      'for names the policy itself writes', () {
    // Names that meet only after the policy has relabelled someone: an
    // owner prefix, the agent mark, and a key suffix typed as a name.
    final tails = [
      for (var i = 0; i < 14; i++)
        fullNpub(key(i))!.substring(fullNpub(key(i))!.length - 4),
    ];
    final random = Random(9699);
    T any<T>(List<T> of) => of[random.nextInt(of.length)];
    String? name() => any([
      'Sam',
      'Sam',
      'Sam',
      ' Sam ',
      'Pat',
      'Pat',
      'Sam (agent)',
      'Pat’s Sam',
      'Pat’s Sam (agent)',
      'Sam · ${any(tails)}',
      'Sam (agent) · ${any(tails)}',
      'Pat’s Sam · ${any(tails)}',
      'Quinn',
      '',
      '\u0085',
      ' \u0085 ',
      null,
    ]);
    var qualified = 0;
    var scoped = 0;
    for (var round = 0; round < 4000; round++) {
      final members = [
        for (var i = 0; i < 2 + random.nextInt(9); i++)
          member(random.nextInt(12), name(), bot: random.nextInt(4) == 0),
      ];
      String someone() => key(random.nextInt(14));
      final owners = {
        for (var i = 0; i < random.nextInt(5); i++) someone(): someone(),
      };
      final sources = IdentityNameSources(
        profiles: {
          for (var i = 0; i < random.nextInt(8); i++)
            someone(): UserProfile(
              pubkey: '',
              displayName: name(),
              ownerPubkey: random.nextInt(4) == 0 ? someone() : null,
            ),
        },
        verifiedNames: {
          for (var i = 0; i < random.nextInt(3); i++) someone(): name() ?? '',
        },
        agentPubkeys: {for (var i = 0; i < random.nextInt(3); i++) someone()},
        agentDisplayNames: {
          for (var i = 0; i < random.nextInt(3); i++) someone(): name() ?? '',
        },
        agentOwners: owners,
        viewer: random.nextBool() ? someone() : null,
      );
      final chips = [
        for (var i = 0; i < 1 + random.nextInt(3); i++)
          MentionCandidate(
            pubkey: random.nextBool() ? someone() : someone().toUpperCase(),
            displayName: name(),
            isAgent: random.nextInt(3) == 0,
            ownerPubkey: random.nextInt(4) == 0 ? someone() : null,
          ),
      ];
      final expected = _amongTheWholeRoster(sources, members, owners, chips);
      final before = mentionQualifierRosterPasses;
      expect(
        mentionChipQualifiers(
          sources: sources,
          members: members,
          userCache: sources.profiles,
          agentOwners: owners,
          chips: chips,
        ),
        expected,
        reason: 'round $round',
      );
      if (expected.isNotEmpty) qualified++;
      if (mentionQualifierRosterPasses - before == 1) scoped++;
    }
    // The rounds reach both outcomes, and the walk often ends after one
    // pass, so the comparison is not of two whole-roster runs.
    expect(qualified, greaterThan(500));
    expect(scoped, greaterThan(500));
  });

  test('chip labels stay those of the whole roster across incremental '
      'changes to the naming facts', () {
    final tails = [
      for (var i = 0; i < 14; i++)
        fullNpub(key(i))!.substring(fullNpub(key(i))!.length - 4),
    ];
    final random = Random(5986525865);
    T any<T>(List<T> of) => of[random.nextInt(of.length)];
    String? name() => any([
      'Sam',
      'Sam',
      ' Sam ',
      'Pat',
      'Pat',
      'Sam (agent)',
      'Pat’s Sam',
      'Kim’s Sam',
      'Pat’s Sam (agent)',
      'Sam · ${any(tails)}',
      'Pat’s Sam · ${any(tails)}',
      'Quinn',
      'Robin',
      'Kim',
      'Lee',
      '',
      ' \u0085 ',
      null,
    ]);
    String someone() => key(random.nextInt(14));
    var skipped = 0;
    var walked = 0;
    var relabelled = 0;
    for (var round = 0; round < 1500; round++) {
      var members = [
        for (var i = 0; i < 2 + random.nextInt(9); i++)
          member(random.nextInt(12), name(), bot: random.nextInt(5) == 0),
      ];
      var profiles = <String, UserProfile>{};
      var verified = <String, String>{};
      var agents = <String>{};
      var directory = <String, String>{};
      var owners = <String, String>{};
      String? viewer;
      // One published change, as the providers make it: a new map or set
      // with the other entries carried over in place.
      void change() {
        switch (random.nextInt(16)) {
          case < 7:
            profiles = {
              ...profiles,
              someone(): UserProfile(
                pubkey: '',
                displayName: name(),
                ownerPubkey: random.nextInt(5) == 0 ? someone() : null,
              ),
            };
          case 7:
            profiles = {...profiles}..remove(someone());
          case 8 || 9:
            verified = {...verified, someone(): name() ?? ''};
          case 10:
            verified = {...verified}..remove(someone());
          case 11:
            directory = {...directory, someone(): name() ?? ''};
          case 12:
            owners = random.nextBool()
                ? {...owners, someone(): someone()}
                : ({...owners}..remove(someone()));
          case 13:
            final agent = someone();
            agents = agents.contains(agent)
                ? ({...agents}..remove(agent))
                : {...agents, agent};
          case 14:
            // Several entries at once, and a map out of step with the last.
            profiles = {
              for (final entry in profiles.entries.toList().reversed)
                if (random.nextInt(4) != 0) entry.key: entry.value,
              someone(): UserProfile(pubkey: '', displayName: name()),
            };
          default:
            if (random.nextBool()) {
              viewer = random.nextBool() ? someone() : null;
            } else {
              members = [...members];
            }
        }
      }

      for (var i = 0; i < random.nextInt(8); i++) {
        change();
      }
      final chips = [
        for (var i = 0; i < 1 + random.nextInt(2); i++)
          MentionCandidate(
            pubkey: random.nextBool() ? someone() : someone().toUpperCase(),
            displayName: name(),
            isAgent: random.nextInt(4) == 0,
            ownerPubkey: random.nextInt(5) == 0 ? someone() : null,
          ),
      ];
      final labeller = MentionChipLabeller();
      Map<String, String>? last;
      for (var step = 0; step < 10; step++) {
        if (step > 0) change();
        final sources = IdentityNameSources(
          profiles: profiles,
          verifiedNames: {...verified},
          agentPubkeys: agents,
          agentDisplayNames: directory,
          agentOwners: owners,
          viewer: viewer,
        );
        final expected = _amongTheWholeRoster(sources, members, owners, chips);
        final before = mentionQualifierRosterPasses;
        expect(
          labeller.labels(
            sources: sources,
            members: members,
            userCache: profiles,
            agentOwners: owners,
            chips: chips,
          ),
          expected,
          reason: 'round $round step $step',
        );
        if (step > 0) {
          mentionQualifierRosterPasses == before ? skipped++ : walked++;
          if (!mapEquals(last, expected)) relabelled++;
        }
        last = expected;
      }
    }
    // The steps reach every outcome: changes that cannot touch a chip and
    // skip the walk, changes that walk, and changes that relabel a chip.
    expect(skipped, greaterThan(2000));
    expect(walked, greaterThan(2000));
    expect(relabelled, greaterThan(300));
  });

  test('renaming the owner of agents outside the chip\'s namesakes '
      'relabels the chip', () {
    // Two agents named Sam are told apart by their owners' names. Neither
    // meets the picked agent `Pat’s Sam` until an owner is renamed Pat.
    final members = [
      member(1, 'Sam', bot: true),
      member(2, 'Sam', bot: true),
      member(3, 'Quinn'),
    ];
    final owners = {key(1): key(5), key(2): key(6)};
    final chips = [
      MentionCandidate(
        pubkey: key(13),
        displayName: 'Pat’s Sam',
        isAgent: true,
      ),
    ];
    final labeller = MentionChipLabeller();
    var profiles = <String, UserProfile>{
      key(5): const UserProfile(pubkey: '', displayName: 'Kim'),
      key(6): const UserProfile(pubkey: '', displayName: 'Lee'),
    };
    Map<String, String> labels() {
      final sources = IdentityNameSources(
        profiles: profiles,
        agentOwners: owners,
      );
      final labels = labeller.labels(
        sources: sources,
        members: members,
        userCache: profiles,
        agentOwners: owners,
        chips: chips,
      );
      expect(labels, _amongTheWholeRoster(sources, members, owners, chips));
      return labels;
    }

    expect(labels(), isEmpty);
    var walked = mentionQualifierRosterPasses;
    profiles = {
      ...profiles,
      key(9): const UserProfile(pubkey: '', displayName: 'Robin'),
    };
    expect(labels(), isEmpty);
    expect(mentionQualifierRosterPasses, walked);
    profiles = {
      ...profiles,
      key(5): const UserProfile(pubkey: '', displayName: 'Pat'),
    };
    expect(labels(), isNotEmpty);
    walked = mentionQualifierRosterPasses;
    profiles = {
      ...profiles,
      key(5): const UserProfile(pubkey: '', displayName: 'Kim'),
    };
    expect(labels(), isEmpty);
    expect(mentionQualifierRosterPasses, greaterThan(walked));
  });

  test('a draft without a chip reads no member', () {
    final before = mentionQualifierRosterPasses;
    expect(
      mentionChipQualifiers(
        sources: IdentityNameSources.empty,
        members: _Unreadable(),
        userCache: const {},
        agentOwners: const {},
        chips: const [],
      ),
      isEmpty,
    );
    expect(mentionQualifierRosterPasses, before);
  });
}

/// A roster that fails the test when anything reads it.
class _Unreadable implements List<ChannelMember> {
  @override
  dynamic noSuchMethod(Invocation invocation) =>
      fail('read the roster: ${invocation.memberName}');
}
