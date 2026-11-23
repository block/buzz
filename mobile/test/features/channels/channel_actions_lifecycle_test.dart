import 'dart:async';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_actions_sheet.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channel_sections/channel_sections_provider.dart';
import 'package:buzz/features/channels/channel_sections/channel_sections_storage.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

const _currentPubkey = 'me';

Channel _channel({
  String type = 'stream',
  bool isArchived = false,
  bool isMember = true,
}) => Channel(
  id: 'channel-id',
  name: type == 'dm' ? 'Alice' : 'general',
  channelType: type,
  visibility: 'open',
  description: '',
  createdBy: 'owner',
  createdAt: DateTime(2025),
  memberCount: 2,
  isMember: isMember,
  archivedAt: isArchived ? DateTime(2025, 1, 2) : null,
);

void main() {
  for (final native in [false, true]) {
    testWidgets('archive returns to an interactive list (native: $native)', (
      tester,
    ) async {
      var archive = Completer<void>();
      Completer<String?>? selection;
      var opened = 0;
      bool? result;
      const bridge = MethodChannel('buzz/channel_actions_sheet');
      if (native) {
        debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
        tester.view.devicePixelRatio = 1;
        tester.view.physicalSize = const Size(951, 669);
        tester.view.padding = const FakeViewPadding(right: 76);
        tester.view.viewPadding = tester.view.padding;
        addTearDown(tester.view.reset);
        addTearDown(() => debugDefaultTargetPlatformOverride = null);
        tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(bridge, (
          call,
        ) async {
          if (call.method != 'present') return null;
          opened++;
          selection = Completer<String?>();
          return selection!.future;
        });
        addTearDown(
          () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
            bridge,
            null,
          ),
        );
      }
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            currentPubkeyProvider.overrideWith((ref) => _currentPubkey),
            channelMembersProvider('channel-id').overrideWith(
              (ref) async => [
                ChannelMember(
                  pubkey: _currentPubkey,
                  role: 'owner',
                  joinedAt: DateTime(2025),
                ),
              ],
            ),
            channelSectionsProvider.overrideWith(
              () => _FakeChannelSectionsNotifier(const ChannelSectionStore()),
            ),
            agentOwnersProvider.overrideWithValue(
              const AsyncValue.data(<String, String>{}),
            ),
            channelActionsProvider.overrideWith(
              (ref) => _FakeChannelActions(
                ref,
                onArchiveChannel: () => archive.future,
              ),
            ),
          ],
          child: MaterialApp(
            theme: AppTheme.light(),
            home: Builder(
              builder: (context) => Scaffold(
                body: TextButton(
                  onPressed: () async {
                    result = await showChannelDetailsSheet(
                      context: context,
                      channel: _channel(),
                      currentPubkey: _currentPubkey,
                      onMemberTap: (_, _) {},
                      personalChat: true,
                    );
                  },
                  child: const Text('Open settings'),
                ),
              ),
            ),
          ),
        ),
      );

      Future<void> confirmArchive() async {
        await tester.tap(find.text('Open settings'));
        await tester.pumpAndSettle();
        if (native) {
          selection!.complete('Archive channel');
          await tester.pumpAndSettle();
        } else {
          await tester.scrollUntilVisible(
            find.text('Archive channel'),
            200,
            scrollable: find.byType(Scrollable).last,
          );
          await tester.tap(find.text('Archive channel'));
        }
        await tester.pumpAndSettle();
        await tester.tap(find.text('Archive'));
        await tester.pumpAndSettle();
      }

      // Closing the sheet while the archive is still in flight leaves Home in
      // place once the archive completes.
      await confirmArchive();
      if (native) {
        expect(result, isNull);
        await tester.tap(find.text('Open settings'));
        await tester.pumpAndSettle();
        expect(opened, 2);
        selection!.complete(null);
      } else {
        expect(find.text('Archive channel'), findsOneWidget);
        await tester.tap(find.byTooltip('Close sheet'));
      }
      await tester.pumpAndSettle();
      archive.complete();
      await tester.pumpAndSettle();
      expect(find.text('Open settings'), findsOneWidget);

      // Left open, the sheet closes itself once the archive succeeds.
      archive = Completer<void>();
      await confirmArchive();
      archive.complete();
      await tester.pumpAndSettle();
      expect(find.text('Archive channel'), findsNothing);
      expect(find.text('Open settings'), findsOneWidget);
      expect(result, isTrue);
      debugDefaultTargetPlatformOverride = null;
    });
  }
}

class _FakeChannelSectionsNotifier extends ChannelSectionsNotifier {
  _FakeChannelSectionsNotifier(this._store);

  final ChannelSectionStore _store;

  @override
  ChannelSectionsState build() =>
      ChannelSectionsState(isReady: true, store: _store, version: 1);
}

class _FakeChannelActions extends ChannelActions {
  final Future<void> Function()? onArchiveChannel;

  _FakeChannelActions(Ref ref, {this.onArchiveChannel})
    : super(
        ref: ref,
        session: ref.read(relaySessionProvider.notifier),
        signedEventRelay: SignedEventRelay(
          session: ref.read(relaySessionProvider.notifier),
          nsec: null,
        ),
        currentPubkey: _currentPubkey,
      );

  @override
  Future<void> archiveChannel(String channelId) async {
    await onArchiveChannel?.call();
  }
}
