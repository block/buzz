import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_detail_page.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/compose_bar.dart';
import 'package:buzz/features/profile/user_profile_sheet.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  final peer = 'a' * 64;

  testWidgets('Message opens the DM without waiting on Solo', (tester) async {
    final dm = Channel(
      id: 'dm-peer',
      name: 'Peer',
      channelType: 'dm',
      visibility: 'private',
      description: '',
      memberCount: 2,
      createdBy: 'self',
      createdAt: DateTime(2025),
      isMember: true,
    );
    tester.view.physicalSize = const Size(390, 844);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final opened = <List<String>>[];
    final waits = <bool>[];
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          userCacheProvider.overrideWith(
            () => _UserCache({
              peer: UserProfile(pubkey: peer, displayName: 'Peer'),
            }),
          ),
          channelActionsProvider.overrideWith(
            (ref) => _FakeDmActions(dm, opened, waits),
          ),
          savedPrefsProvider.overrideWithValue(prefs),
          // Solo organization is still loading.
          meGroupsProvider.overrideWith((ref) => const Stream.empty()),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: Builder(
            builder: (context) => Scaffold(
              body: TextButton(
                onPressed: () => showUserProfileSheet(context, peer),
                child: const Text('Open profile'),
              ),
            ),
          ),
        ),
      ),
    );
    await tester.tap(find.text('Open profile'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Message'));
    await tester.pump();
    await tester.pump(const Duration(seconds: 1));

    expect(opened, [
      [peer],
    ]);
    // The page opens at once instead of waiting on the full channel list.
    expect(waits, [false]);
    expect(
      tester.widget<ChannelDetailPage>(find.byType(ChannelDetailPage)).channel,
      dm,
    );
    // The DM is usable at once, not a placement spinner.
    expect(find.byType(ComposeBar), findsOneWidget);
  });
}

class _UserCache extends UserCacheNotifier {
  _UserCache(this._initial);

  final Map<String, UserProfile> _initial;

  @override
  Map<String, UserProfile> build() => _initial;

  @override
  Future<bool> preload(List<String> pubkeys) async => true;
}

class _FakeDmActions implements ChannelActions {
  _FakeDmActions(this.dm, this.opened, this.waits);

  final Channel dm;
  final List<List<String>> opened;
  final List<bool> waits;

  @override
  Future<Channel> openDm({
    required List<String> pubkeys,
    bool waitForChannelList = true,
  }) async {
    opened.add(pubkeys);
    waits.add(waitForChannelList);
    return dm;
  }

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}
