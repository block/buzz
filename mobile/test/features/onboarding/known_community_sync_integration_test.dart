import 'package:buzz/features/channels/channels_page.dart';
import 'package:buzz/shared/community/community_icon_provider.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import '../../helpers/widget_helpers.dart';
import 'package:buzz/shared/auth/auth_provider.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/community/community_storage.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:buzz/shared/hosted/known_community_sync.dart';
import 'package:buzz/shared/hosted/known_community_sync_provider.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../../shared/community/community_storage_test.dart'
    show FakeSecureStorage;

class _Account extends BuilderlabAccountClient {
  final changes = ValueNotifier<String?>('owner');
  final calls = <String>[];
  @override
  ValueListenable<String?> get subjectChanges => changes;
  @override
  BuilderlabAccount get account => BuilderlabAccount(
    subject: 'owner',
    expiresAt: DateTime.now().add(const Duration(hours: 1)),
  );
  @override
  Future<BuilderlabAccount?> restore() async => account;
  @override
  Future<Map<String, dynamic>> post(
    String path,
    Map<String, dynamic> body, {
    String? expectedSubject,
  }) async {
    expectSync(expectedSubject, 'owner');
    if (path.endsWith('/list')) {
      return {
        'communities': [
          {'community_url': 'wss://paired.example'},
        ],
      };
    }
    expectSync(body['community_url'], 'wss://paired.example');
    calls.add(path.split('/').last);
    return {};
  }
}

void main() {
  testWidgets('removing an account-only destination hides its queued removal', (
    tester,
  ) async {
    if (!HostedEnvironment.enabled) return;
    final account = _Account();
    final storage = CommunityStorage(secure: FakeSecureStorage());
    final sync = KnownCommunitySync(
      account: account,
      storage: FakeSecureStorage(),
      localRelays: () async => [],
    );
    // Keep the server response stale so the durable removal filter is exercised.
    sync.setActive(false);
    addTearDown(sync.dispose);
    await tester.pumpWidget(
      WidgetHelpers.testable(
        enableAccountCommunitySync: true,
        overrides: [
          builderlabAccountProvider.overrideWithValue(account),
          communityStorageProvider.overrideWithValue(storage),
          knownCommunitySyncProvider.overrideWithValue(sync),
          communityIconPresentationProvider.overrideWith((ref, relay) => null),
        ],
        child: CommunitySwitcherPage(
          destination: null,
          prepareLanding: () async => null,
          onFlightChanged: (_) {},
          onTransitionProgress: (_) {},
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('paired.example'), findsOneWidget);
    await tester.tap(find.text('Edit'));
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Remove paired.example'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Remove').last);
    await tester.pumpAndSettle();
    expect(find.text('paired.example'), findsNothing);
    expect(await sync.pendingRemovals('owner'), {'wss://paired.example'});
    expect(account.calls, isEmpty);
    sync.setActive(true);
    await tester.pumpAndSettle();
    expect(account.calls, ['remove']);
    expect(await storage.loadAll(), isEmpty);
    expect(tester.takeException(), isNull);
  });

  test(
    'shared authentication and explicit removal enqueue account edits',
    () async {
      final account = _Account();
      final storage = CommunityStorage(secure: FakeSecureStorage());
      final sync = KnownCommunitySync(
        account: account,
        storage: FakeSecureStorage(),
        localRelays: () async =>
            (await storage.loadAll()).map((c) => c.relayUrl).toList(),
      );
      final container = ProviderContainer(
        overrides: [
          builderlabAccountProvider.overrideWithValue(account),
          communityStorageProvider.overrideWithValue(storage),
          knownCommunitySyncProvider.overrideWithValue(sync),
          communitySnapshotWriterProvider.overrideWithValue((_) async {}),
          communityPushLeaseRevocationEnqueuerProvider.overrideWithValue(
            (_) async => false,
          ),
        ],
      );
      addTearDown(() {
        container.dispose();
        sync.dispose();
      });
      final community = Community.create(
        name: 'Paired',
        relayUrl: 'wss://paired.example',
        nsec: nostr.Keys('1'.padLeft(64, '0')).nsec,
      );
      await container.read(communityListProvider.future);
      // Pairing, invites, and self-hosted entry all persist through this seam.
      await container
          .read(authProvider.notifier)
          .authenticateWithCommunity(community);
      await sync.synchronize();
      expect(account.calls, ['add']);
      await container.read(communityListProvider.future);
      await container
          .read(communityListProvider.notifier)
          .removeCommunity(community.id);
      await sync.synchronize();
      expect(account.calls, ['add', 'remove']);
      expect(await storage.loadAll(), isEmpty);
    },
  );
}
