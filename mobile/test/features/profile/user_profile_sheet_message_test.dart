import 'dart:async';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_actions_sheet.dart';
import 'package:buzz/features/channels/channel_detail_page.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/compose_bar.dart';
import 'package:buzz/features/profile/user_profile_sheet.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  final peer = 'a' * 64;

  for (final nativeSheet in [false, true]) {
    testWidgets(
      'Message opens the DM without waiting on Solo (native sheet: $nativeSheet)',
      (tester) async {
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
        final solo = Channel(
          id: 'solo-project',
          name: 'Project',
          channelType: 'group',
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
        addTearDown(() => debugDefaultTargetPlatformOverride = null);
        const bridge = MethodChannel('buzz/channel_actions_sheet');
        Completer<String?>? nativeResult;
        var presentations = 0;
        tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(bridge, (
          call,
        ) async {
          if (call.method == 'present') {
            presentations++;
            nativeResult = Completer<String?>();
            return nativeResult!.future;
          }
          return null;
        });
        addTearDown(
          () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
            bridge,
            null,
          ),
        );
        final opened = <List<String>>[];
        final waits = <bool>[];
        SharedPreferences.setMockInitialValues({});
        final prefs = await SharedPreferences.getInstance();
        await tester.pumpWidget(
          ProviderScope(
            overrides: [
              currentPubkeyProvider.overrideWith((ref) => 'self'),
              agentOwnersProvider.overrideWithValue(
                const AsyncValue.data(<String, String>{}),
              ),
              channelMembersProvider(solo.id).overrideWith(
                (ref) async => [
                  ChannelMember(
                    pubkey: peer,
                    role: 'member',
                    joinedAt: DateTime(2025),
                  ),
                ],
              ),
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
                    onPressed: () => nativeSheet
                        ? showChannelDetailsSheet(
                            context: context,
                            channel: solo,
                            currentPubkey: 'self',
                            personalChat: true,
                            onMemberTap: (context, pubkey) =>
                                showUserProfileSheet(context, pubkey),
                          )
                        : showUserProfileSheet(context, peer),
                    child: const Text('Open profile'),
                  ),
                ),
              ),
            ),
          ),
        );
        Future<void> openProfile() async {
          if (nativeSheet) {
            tester.view.physicalSize = const Size(1200, 800);
            tester.view.padding = const FakeViewPadding(right: 76);
            tester.view.viewPadding = tester.view.padding;
            debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
            await tester.pumpAndSettle();
          }
          await tester.tap(find.text('Open profile'));
          await tester.pumpAndSettle();
          if (nativeSheet) {
            // UIKit selection is the OS boundary; follow-up profile/DM routes are real.
            debugDefaultTargetPlatformOverride = null;
            expect(nativeResult, isNotNull);
            expect(
              nativeResult!.isCompleted,
              isFalse,
              reason: 'The underlying list must accept another tap',
            );
            nativeResult!.complete('member:$peer');
            await tester.pumpAndSettle();
          }
          expect(find.byType(UserProfileSheet), findsOneWidget);
        }

        await openProfile();
        if (nativeSheet) {
          Navigator.of(tester.element(find.byType(UserProfileSheet))).pop();
          await tester.pumpAndSettle();
          // A stranded transparent owner route prevents this real tap from opening again.
          await openProfile();
          expect(presentations, 2);
        }
        await tester.tap(find.text('Message'));
        await tester.pump();
        await tester.pump(const Duration(seconds: 1));

        expect(opened, [
          [peer],
        ]);
        // The page opens at once instead of waiting on the full channel list.
        expect(waits, [false]);
        expect(
          tester
              .widget<ChannelDetailPage>(find.byType(ChannelDetailPage))
              .channel,
          dm,
        );
        // The DM is usable at once, not a placement spinner.
        expect(find.byType(ComposeBar), findsOneWidget);
        if (nativeSheet) {
          Navigator.of(tester.element(find.byType(ChannelDetailPage))).pop();
          await tester.pumpAndSettle();
          await openProfile();
          expect(presentations, 3);
          Navigator.of(tester.element(find.byType(UserProfileSheet))).pop();
          await tester.pumpAndSettle();
        }
      },
    );
  }
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
