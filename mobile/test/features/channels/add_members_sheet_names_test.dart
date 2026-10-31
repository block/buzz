import 'package:buzz/features/channels/add_members_sheet.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

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
          relayDirectoryUsersProvider.overrideWith(
            (ref) async => [
              DirectoryUser(pubkey: agent, displayName: 'Scout', isAgent: true),
            ],
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

    // The shown agent is told apart from the human member named Scout.
    expect(find.text('Scout (agent)'), findsOneWidget);
    // Only the shown choice's profile is requested, not the whole roster.
    expect(cache.preloaded, {agent});
  });
}
