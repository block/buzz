import 'package:flutter/widgets.dart';
import 'package:buzz/shared/hosted/account_community_sync_status.dart';
import 'dart:async';
import 'dart:convert';
import 'package:buzz/features/pairing/pairing_provider.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import '../../helpers/controllable_pairing_socket.dart';
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
  Object? failure;
  List<String> urls = ['wss://paired.example'];
  final targets = <String>[];
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
        'communities': urls.map((url) => {'community_url': url}).toList(),
      };
    }
    expectSync(urls, contains(body['community_url']));
    targets.add(body['community_url'] as String);
    calls.add(path.split('/').last);
    if (failure case final error?) throw error;
    return {};
  }
}

class _JournalStorage extends FakeSecureStorage {
  bool failWrites = false;
  @override
  Future<void> write({
    required String key,
    required String? value,
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) {
    if (failWrites) throw StateError('Account journal unavailable');
    return super.write(key: key, value: value);
  }
}

void main() {
  for (final localEntries in [false, true]) {
    testWidgets(
      'removing ${localEntries ? 'local' : 'account-only'} ports shows precise pending removal',
      (tester) async {
        if (!HostedEnvironment.enabled) return;
        final account = _Account()
          ..urls = ['wss://team.example:8443', 'wss://team.example:9443'];
        final storage = CommunityStorage(secure: FakeSecureStorage());
        if (localEntries) {
          for (final port in [8443, 9443]) {
            await storage.save(
              Community(
                id: '$port',
                name: 'team.example',
                relayUrl: 'wss://team.example:$port',
                addedAt: DateTime(2025),
              ),
            );
          }
        }
        final label = localEntries
            ? 'team.example (team.example:9443)'
            : 'team.example:9443';
        final sync = KnownCommunitySync(
          account: account,
          storage: FakeSecureStorage(),
          localCommunities: storage.loadForAccountSync,
          acknowledgeRemoval: storage.acknowledgeRemoval,
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
              communitySnapshotWriterProvider.overrideWithValue((_) async {}),
              communityPushLeaseRevocationEnqueuerProvider.overrideWithValue(
                (_) async => false,
              ),
              knownCommunitySyncProvider.overrideWithValue(sync),
              communityIconPresentationProvider.overrideWith(
                (ref, relay) => null,
              ),
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
        expect(find.text(label), findsOneWidget);
        await tester.tap(find.text('Edit'));
        await tester.pumpAndSettle();
        await tester.tap(find.byTooltip('Remove $label'));
        await tester.pumpAndSettle();
        expect(find.textContaining('Remove “$label”'), findsOneWidget);
        expect(
          find.text(
            localEntries
                ? 'team.example (team.example:8443)'
                : 'team.example:8443',
          ),
          findsOneWidget,
        );
        await tester.tap(find.text('Remove').last);
        await tester.pumpAndSettle();
        expect(find.text('team.example:9443'), findsOneWidget);
        expect(find.textContaining('is pending account sync'), findsOneWidget);
        expect(await sync.pendingRemovals('owner'), {
          'wss://team.example:9443',
        });
        expect(account.calls, isEmpty);
        account.failure = AccountFailure.unavailable;
        sync.setActive(true);
        await tester.pumpAndSettle();
        expect(find.textContaining('hasn’t synced'), findsOneWidget);
        // A refused endpoint remains visible and offers explicit retry.
        account.failure = AccountFailure.rejected;
        await tester.tap(find.text('Retry account sync'));
        await tester.pumpAndSettle();
        expect(find.textContaining('hasn’t synced'), findsOneWidget);
        await tester.pumpWidget(const SizedBox.shrink());
        // The service remains scoped to this signed-in account across picker opens.
        await tester.pumpWidget(
          WidgetHelpers.testable(
            enableAccountCommunitySync: true,
            overrides: [
              builderlabAccountProvider.overrideWithValue(account),
              knownCommunitySyncProvider.overrideWithValue(sync),
            ],
            child: const AccountCommunitySyncStatus(),
          ),
        );
        await tester.pumpAndSettle();
        expect(find.textContaining('hasn’t synced'), findsOneWidget);
        account.failure = null;
        await tester.tap(find.text('Retry account sync'));
        await tester.pumpAndSettle();
        expect(find.textContaining('hasn’t synced'), findsNothing);
        expect(account.calls, ['remove', 'remove', 'remove']);
        expect(account.targets, everyElement('wss://team.example:9443'));
        expect(
          (await storage.loadAll()).map((c) => c.id),
          localEntries ? ['8443'] : <String>[],
        );
        expect(tester.takeException(), isNull);
      },
    );
  }

  test(
    'pairing acknowledges committed re-add despite journal failure and restart',
    () async {
      const sourceSecret =
          '09b3065e3570a3a4054660dccd66e12774a99a904fdb0ca02dbc6c3136249506';
      const sessionSecret =
          'abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789';
      final account = _Account();
      final storage = CommunityStorage(secure: FakeSecureStorage());
      final journal = _JournalStorage();
      var sync = KnownCommunitySync(
        account: account,
        storage: journal,
        localCommunities: storage.loadForAccountSync,
        acknowledgeRemoval: storage.acknowledgeRemoval,
      );
      await sync.remove('wss://paired.example');
      journal.failWrites = true;
      late ControllablePairingSocket socket;
      final notifier = PairingNotifier(
        credentialValidator:
            ({required String relayUrl, required String? nsec}) async {},
        socketFactory:
            ({
              required wsUrl,
              required ephemeralPrivkey,
              required onMessage,
              required onDisconnected,
            }) {
              socket = ControllablePairingSocket(
                ephemeralPrivkey: ephemeralPrivkey,
                onMessage: onMessage,
                onDisconnected: onDisconnected,
              );
              return socket;
            },
      );
      final container = ProviderContainer(
        overrides: [
          builderlabAccountProvider.overrideWithValue(account),
          communityStorageProvider.overrideWithValue(storage),
          knownCommunitySyncProvider.overrideWithValue(sync),
          pairingProvider.overrideWith(() => notifier),
          communitySnapshotWriterProvider.overrideWithValue((_) async {}),
        ],
      );
      container.read(pairingProvider);
      final ready = Completer<void>();
      final finished = Completer<void>();
      container.listen(pairingProvider, (_, next) {
        if (next.status == PairingStatus.transferring && !ready.isCompleted) {
          ready.complete();
        }
        if ((next.status == PairingStatus.success ||
                next.status == PairingStatus.error) &&
            !finished.isCompleted) {
          finished.complete();
        }
      });
      await notifier.pair(
        'nostrpair://${nostr.Keys(sourceSecret).public}?secret=$sessionSecret&relay=wss%3A%2F%2Fpairing.buzz.xyz&v=1',
      );
      notifier.setProtectSensitiveActions(false);
      notifier.confirmSas();
      socket.sendSourceMessage(
        sourceSecret: sourceSecret,
        sessionSecretHex: sessionSecret,
        message: {'type': 'sas-confirm'},
        includeTranscriptHash: true,
      );
      await ready.future;
      socket.sendSourceMessage(
        sourceSecret: sourceSecret,
        sessionSecretHex: sessionSecret,
        message: {
          'type': 'payload',
          'payload_type': 'credentials',
          'payload': jsonEncode({
            'relayUrl': 'https://paired.example',
            'pubkey': nostr.Keys(sourceSecret).public,
            'nsec': nostr.Keys(sourceSecret).nsec,
          }),
        },
      );
      await finished.future;
      await sync.synchronize();
      expect(container.read(pairingProvider).status, PairingStatus.success);
      final acknowledgements = socket
          .decryptedPublishedMessages(sourceSecret)
          .where((m) => m['type'] == 'complete')
          .toList();
      expect(acknowledgements.single['success'], isTrue);
      final saved = (await storage.loadAll()).single;
      expect(saved.accountSyncOwner, endsWith('\nowner'));
      expect(saved.accountSyncRevision, isNotNull);
      expect(await sync.pendingRemovals('owner'), {'wss://paired.example'});
      // Recreate the process after local commit, before journal reconciliation.
      container.dispose();
      sync.dispose();
      journal.failWrites = false;
      sync = KnownCommunitySync(
        account: account,
        storage: journal,
        localCommunities: storage.loadForAccountSync,
        acknowledgeRemoval: storage.acknowledgeRemoval,
      );
      addTearDown(sync.dispose);
      await sync.synchronize();
      expect(account.calls, ['add']);
      expect(await sync.pendingRemovals('owner'), isEmpty);
      expect((await storage.loadAll()).single.nsec, saved.nsec);
    },
  );

  test(
    'shared authentication and explicit removal enqueue account edits',
    () async {
      final account = _Account();
      final storage = CommunityStorage(secure: FakeSecureStorage());
      final sync = KnownCommunitySync(
        account: account,
        storage: FakeSecureStorage(),
        localCommunities: storage.loadForAccountSync,
        acknowledgeRemoval: storage.acknowledgeRemoval,
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
