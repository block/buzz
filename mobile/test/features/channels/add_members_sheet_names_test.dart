import 'dart:async';

import 'package:buzz/features/channels/add_members_sheet.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/verified_profiles/verified_profile.dart';
import 'package:buzz/shared/verified_profiles/verified_profile_discovery_fetch.dart';
import 'package:buzz/shared/verified_profiles/verified_profile_proof.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../helpers/recipient_directory_session.dart';
import '../../shared/verified_profiles/verified_profile_fixture.dart';

class _RecordingUserCache extends UserCacheNotifier {
  final preloaded = <String>{};

  @override
  Map<String, UserProfile> build() => const {};

  @override
  UserProfile? get(String pubkey) => null;

  @override
  Future<bool> preload(List<String> pubkeys) async {
    preloaded.addAll(pubkeys);
    return true;
  }
}

/// A relay whose profile search finds [heads] alone: people with a verified
/// profile and no kind:0.
class _VerifiedOnlySession extends RecipientDirectorySession {
  _VerifiedOnlySession(this.heads) : super(const {});

  final List<NostrEvent> heads;

  @override
  Future<List<NostrEvent>> searchProfiles(
    String query, {
    int limit = 50,
    Duration timeout = const Duration(seconds: 8),
  }) async => heads;
}

class _Actions extends Fake implements ChannelActions {
  final calls = <List<String>>[];
  @override
  Future<void> addMembers({
    required String channelId,
    required List<String> pubkeys,
    String role = 'member',
  }) async => calls.add(pubkeys);
}

void main() {
  testWidgets('compares with every member but loads only shown choices', (
    tester,
  ) async {
    const channelId = 'channel-1';
    // A large roster with no cached profiles; one human member is Scout.
    final roster = [
      for (var i = 0; i < 200; i++)
        ChannelMember(
          pubkey: i.toRadixString(16).padLeft(64, '0'),
          role: 'member',
          joinedAt: DateTime(2026),
          displayName: i == 0 ? 'Scout' : 'Member $i',
        ),
    ];
    final agent = 'f' * 64;
    final cache = _RecordingUserCache();

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          userCacheProvider.overrideWith(() => cache),
          channelMembersProvider(channelId).overrideWith((ref) async => roster),
          relaySessionProvider.overrideWith(
            () => RecipientDirectorySession({agent: 'Scout'}, agents: {agent}),
          ),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: AddChannelMembersSheet(
              channelId: channelId,
              existingPubkeys: {for (final member in roster) member.pubkey},
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    await tester.enterText(
      find.byKey(const ValueKey('add-channel-members-search')),
      'Scout',
    );
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 300));
    await tester.pumpAndSettle();

    // The shown agent is told apart from the human member named Scout.
    expect(find.text('Scout (agent)'), findsOneWidget);
    // Only the shown choice's profile is requested, not the whole roster.
    expect(cache.preloaded, {agent});
  });

  testWidgets(
    'finds a verified person with no profile by full name once discovery ends',
    (tester) async {
      // The real shape: the display name is the username.
      final gabi = signedProjection(
        profileTags(
          subjectA,
          name: 'glilienfeld',
          username: 'glilienfeld',
          fullName: 'Gabi Lilienfeld',
        ),
      );
      // Neither of these finds anyone: another key's head, and a head that
      // claims the relay key with a signature it never made.
      final otherKey = signedProjection(
        profileTags(subjectB, name: 'gabi_other'),
        author: relayKeysOther(),
      );
      final claimed = signedProjection(
        profileTags('c' * 64, name: 'gabi_forged'),
      );
      final forged = NostrEvent.fromJson({
        ...claimed.toJson(),
        'sig': '0' * 128,
      });
      final discovery = Completer<VerifiedProfileDiscovery>();
      final actions = _Actions();
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            userCacheProvider.overrideWith(_RecordingUserCache.new),
            channelActionsProvider.overrideWithValue(actions),
            channelMembersProvider('channel').overrideWith((ref) async => []),
            relaySessionProvider.overrideWith(
              () => _VerifiedOnlySession([gabi, otherKey, forged]),
            ),
            verifiedProfileDiscoveryFetchProvider.overrideWith(
              (ref) =>
                  () => discovery.future,
            ),
            verifiedProfileProofCheckerProvider.overrideWithValue(
              (events) async => [
                for (final e in events) hasVerifiedEventProof(e),
              ],
            ),
          ],
          child: MaterialApp(
            theme: AppTheme.light(),
            home: const Scaffold(
              body: AddChannelMembersSheet(
                channelId: 'channel',
                existingPubkeys: {},
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      await tester.enterText(
        find.byKey(const ValueKey('add-channel-members-search')),
        'gabi',
      );
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 300));
      // The search waits for the relay to say who signs verified profiles.
      expect(find.text('glilienfeld'), findsNothing);
      discovery.complete(VerifiedProfilesEnabled(authority));
      await tester.pumpAndSettle();

      expect(find.text('glilienfeld'), findsOneWidget);
      expect(find.textContaining('gabi_'), findsNothing);
      await tester.tap(find.text('glilienfeld'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Add member'));
      await tester.pumpAndSettle();
      expect(actions.calls, [
        [subjectA],
      ]);
    },
  );
}
