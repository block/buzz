import 'dart:math';

import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/compose_bar.dart';
import 'package:buzz/features/channels/mentions/mention_candidates.dart';
import 'package:buzz/features/channels/mentions/mention_candidates_provider.dart';
import 'package:buzz/features/channels/mentions/mention_ranking.dart';
import 'package:buzz/shared/identity_names/identity_names.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/utils/string_utils.dart';
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
