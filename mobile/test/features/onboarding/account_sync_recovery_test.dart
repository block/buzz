import 'dart:async';
import 'dart:convert';

import 'package:buzz/shared/auth/auth_provider.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/community/community_storage.dart';
import 'package:buzz/shared/hosted/account_community_sync_status.dart';
import 'package:buzz/shared/hosted/account_identity_store.dart';
import 'package:buzz/shared/hosted/browser_auth.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/hosted_flow.dart';
import 'package:buzz/shared/hosted/hosted_flow_provider.dart';
import 'package:buzz/shared/hosted/hosted_identity_service.dart';
import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:buzz/shared/hosted/known_community_sync.dart';
import 'package:buzz/shared/hosted/known_community_sync_provider.dart';
import 'package:flutter/material.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../../helpers/hosted_communities_fixture.dart';
import '../../helpers/widget_helpers.dart';
import '../../shared/community/community_storage_test.dart'
    show FakeSecureStorage;

class _Backend {
  final expiry = DateTime.now()
      .add(const Duration(days: 1))
      .toUtc()
      .toIso8601String();
  String subject = 'A';
  bool offline = false;
  bool denied = false;
  Completer<void>? restoreGate;
  final restoreStarted = Completer<void>();
  final edits = <({String subject, String op, String url})>[];
  final sessions = FakeSecureStorage();
  BuilderlabAccountClient client() => BuilderlabAccountClient(
    storage: sessions,
    httpClient: MockClient((request) async {
      final path = request.url.path;
      if (path.endsWith('/me') && restoreGate != null) {
        if (!restoreStarted.isCompleted) restoreStarted.complete();
        await restoreGate!.future;
      }
      if (offline) return http.Response('{}', 503);
      if (path.endsWith('/exchange')) {
        return http.Response(
          jsonEncode({'session_credential': subject, 'expires_at': expiry}),
          200,
        );
      }
      if (path.endsWith('/me')) {
        return http.Response(
          jsonEncode({
            'subject': request.headers['X-BB-Session-Credential'],
            'expires_at': expiry,
          }),
          200,
        );
      }
      if (path.endsWith('/list')) {
        return http.Response('{"communities":[]}', 200);
      }
      if (path.contains('known-communities')) {
        final body = jsonDecode(request.body) as Map<String, dynamic>;
        if (denied) return http.Response('{}', 403);
        edits.add((
          subject: request.headers['X-BB-Session-Credential']!,
          op: path.split('/').last,
          url: body['community_url'] as String,
        ));
        return http.Response('{}', 200);
      }
      return http.Response('{}', 503);
    }),
  );
}

class _BrokenJournal extends FakeSecureStorage {
  String? broken;
  @override
  Future<String?> read({
    required String key,
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) {
    if (broken == 'read') throw StateError('journal read failed');
    return super.read(key: key);
  }

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
    if (broken == 'write') throw StateError('journal write failed');
    return super.write(key: key, value: value);
  }
}

Community _community(String id, [String url = 'wss://private.example']) =>
    Community(
      id: id,
      name: Uri.parse(url).host,
      relayUrl: url,
      addedAt: DateTime(2025),
      nsec: nostr.Keys('1'.padLeft(64, '0')).nsec,
    );

ProviderContainer _container(
  BuilderlabAccountClient account,
  CommunityStorage storage,
  KnownCommunitySync sync, {
  Future<bool> Function(Community)? beforeRemove,
}) => ProviderContainer(
  overrides: [
    builderlabAccountProvider.overrideWithValue(account),
    communityStorageProvider.overrideWithValue(storage),
    knownCommunitySyncProvider.overrideWithValue(sync),
    communitySnapshotWriterProvider.overrideWithValue((_) async {}),
    communityPushLeaseRevocationEnqueuerProvider.overrideWithValue(
      beforeRemove ?? (_) async => false,
    ),
  ],
);

KnownCommunitySync _sync(
  BuilderlabAccountClient account,
  CommunityStorage storage, {
  FakeSecureStorage? journal,
}) => KnownCommunitySync(
  account: account,
  localCommunities: storage.loadForAccountSync,
  acknowledgeRemoval: storage.acknowledgeRemoval,
  storage: journal ?? FakeSecureStorage(),
);

Future<void> _signIn(BuilderlabAccountClient client) =>
    client.exchange(code: 'code', verifier: 'a' * 43);

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUp(() => FlutterSecureStorage.setMockInitialValues({}));

  test(
    'confirmed removal stays with A across delayed push cleanup and A to B',
    () async {
      final backend = _Backend();
      final account = backend.client();
      await _signIn(account);
      final storage = CommunityStorage(secure: FakeSecureStorage());
      final sync = _sync(account, storage)..setActive(false);
      final entered = Completer<void>();
      final release = Completer<void>();
      var pause = true;
      final container = _container(
        account,
        storage,
        sync,
        beforeRemove: (_) async {
          if (pause) {
            entered.complete();
            await release.future;
          }
          return false;
        },
      );
      addTearDown(() {
        container.dispose();
        sync.dispose();
        account.dispose();
      });
      await storage.save(await sync.prepareAddition(_community('first')));
      await container.read(communityListProvider.future);
      final removing = container
          .read(communityListProvider.notifier)
          .removeCommunity('first');
      final rejected = expectLater(
        removing,
        throwsA(AccountFailure.differentAccount),
      );
      await entered.future;
      await account.clearLocalSession();
      backend.subject = 'B';
      await _signIn(account);
      release.complete();
      await rejected;
      expect((await storage.loadAll()).map((c) => c.id), ['first']);
      sync.setActive(true);
      await sync.synchronize();
      expect(backend.edits, isEmpty);
      await account.clearLocalSession();
      backend.subject = 'A';
      await _signIn(account);
      pause = false;
      await container
          .read(communityListProvider.notifier)
          .removeCommunity(
            'first',
            confirmedSubject: 'A',
            accountConfirmed: true,
          );
      await sync.synchronize();
      expect(await storage.loadAll(), isEmpty);
      expect(backend.edits.where((e) => e.op == 'remove'), [
        (subject: 'A', op: 'remove', url: 'wss://private.example'),
      ]);
      expect(backend.edits.every((e) => e.subject == 'A'), isTrue);
    },
  );

  for (final failure in ['read', 'write']) {
    test(
      'local removal survives $failure failure and duplicate identities, then restart converges',
      () async {
        final backend = _Backend();
        final account = backend.client();
        await _signIn(account);
        final storage = CommunityStorage(secure: FakeSecureStorage());
        final journal = _BrokenJournal();
        var sync = _sync(account, storage, journal: journal)..setActive(false);
        var container = _container(account, storage, sync);
        for (final id in ['first', 'second']) {
          await storage.save(await sync.prepareAddition(_community(id)));
        }
        await container.read(communityListProvider.future);
        journal.broken = failure;
        final firstId = failure == 'read' ? 'first' : 'second';
        final lastId = failure == 'read' ? 'second' : 'first';
        await container
            .read(communityListProvider.notifier)
            .removeCommunity(firstId);
        expect((await storage.loadAll()).map((c) => c.id), [lastId]);
        expect(
          (await storage.loadForAccountSync()).where(
            (c) => c.accountSyncRemoved,
          ),
          isEmpty,
        );
        await container
            .read(communityListProvider.notifier)
            .removeCommunity(lastId);
        expect(await storage.loadAll(), isEmpty);
        final retained = await storage.loadForAccountSync();
        expect(retained.single.accountSyncRemoved, isTrue);
        expect(retained.single.nsec, isNull);
        // Simulate process loss immediately after the atomic local commit.
        container.dispose();
        sync.dispose();
        sync = _sync(account, storage, journal: journal);
        container = _container(account, storage, sync);
        await sync.synchronize();
        expect(backend.edits, [
          (subject: 'A', op: 'remove', url: 'wss://private.example'),
        ]);
        expect(await storage.loadForAccountSync(), isEmpty);
        journal.broken = null;
        sync.retry();
        await sync.synchronize();
        expect(backend.edits, hasLength(1));
        container.dispose();
        sync.dispose();
        account.dispose();
      },
    );
  }

  test(
    'offline restart retains explicit add and remove for A, never sign-out or B',
    () async {
      final backend = _Backend();
      var account = backend.client();
      await _signIn(account);
      final storage = CommunityStorage(secure: FakeSecureStorage());
      var sync = _sync(account, storage)..setActive(false);
      await storage.save(
        await sync.prepareAddition(_community('old', 'wss://old.example')),
      );
      sync.dispose();
      account.dispose();
      account = backend.client();
      backend.offline = true;
      backend.restoreGate = Completer<void>();
      final restoring = expectLater(
        account.restore(),
        throwsA(AccountFailure.unavailable),
      );
      await backend.restoreStarted.future;
      sync = _sync(account, storage)..setActive(false);
      var container = _container(account, storage, sync);
      await container.read(communityListProvider.future);
      await container
          .read(authProvider.notifier)
          .authenticateWithCommunity(
            _community('new', 'wss://new.example'),
            activate: false,
          );
      await container.read(communityListProvider.future);
      await container
          .read(communityListProvider.notifier)
          .removeCommunity('old');
      expect(
        (await storage.loadAll()).single.accountSyncOwner,
        endsWith('\nA'),
      );
      expect(await sync.pendingRemovals('A'), {'wss://old.example'});
      backend.restoreGate!.complete();
      await restoring;
      backend.restoreGate = null;
      container.dispose();
      sync.dispose();
      account.dispose();
      account = backend.client();
      backend.offline = false;
      sync = _sync(account, storage);
      await sync.synchronize();
      expect(backend.edits.toSet(), {
        (subject: 'A', op: 'remove', url: 'wss://old.example'),
        (subject: 'A', op: 'add', url: 'wss://new.example'),
      });
      await account.clearLocalSession();
      container = _container(account, storage, sync);
      await container
          .read(authProvider.notifier)
          .authenticateWithCommunity(
            _community('signed-out', 'wss://unclaimed.example'),
            activate: false,
          );
      expect((await storage.loadAll()).last.accountSyncOwner, isNull);
      backend.subject = 'B';
      await _signIn(account);
      await sync.synchronize();
      expect(backend.edits.where((e) => e.subject == 'B'), isEmpty);
      container.dispose();
      sync.dispose();
      account.dispose();
    },
  );

  testWidgets(
    'interrupted old removal offers explicit local completion and converges',
    (tester) async {
      final backend = _Backend();
      final account = backend.client();
      await _signIn(account);
      final storage = CommunityStorage(secure: FakeSecureStorage());
      final journal = FakeSecureStorage();
      var sync = _sync(account, storage, journal: journal)..setActive(false);
      await storage.save(
        await sync.prepareAddition(
          _community('interrupted', 'https://private.example'),
        ),
      );
      await sync.remove('wss://private.example');
      // Old preview prefix: journal committed but local deletion did not happen.
      sync.dispose();
      sync = _sync(account, storage, journal: journal);
      await sync.synchronize();
      expect(backend.edits, isEmpty);
      await tester.pumpWidget(
        WidgetHelpers.testable(
          enableAccountCommunitySync: true,
          overrides: [
            builderlabAccountProvider.overrideWithValue(account),
            knownCommunitySyncProvider.overrideWithValue(sync),
            communityStorageProvider.overrideWithValue(storage),
            communitySnapshotWriterProvider.overrideWithValue((_) async {}),
            communityPushLeaseRevocationEnqueuerProvider.overrideWithValue(
              (_) async => false,
            ),
          ],
          child: const AccountCommunitySyncStatus(),
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(
        find.text('Finish removing private.example from this device'),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Remove').last);
      await tester.pumpAndSettle();
      await sync.synchronize();
      expect(await storage.loadAll(), isEmpty);
      expect(await sync.pendingRemovals('A'), isEmpty);
      expect(
        backend.edits.every(
          (e) =>
              e.subject == 'A' &&
              e.op == 'remove' &&
              e.url == 'wss://private.example',
        ),
        isTrue,
      );
      expect(backend.edits, isNotEmpty);
      await tester.pumpWidget(const SizedBox.shrink());
      sync.dispose();
      account.dispose();
    },
  );

  testWidgets('pending status drops A immediately while B refresh is delayed', (
    tester,
  ) async {
    final backend = _Backend();
    final account = backend.client();
    await _signIn(account);
    Completer<void>? gate;
    final sync = KnownCommunitySync(
      account: account,
      storage: FakeSecureStorage(),
      localCommunities: () async {
        await gate?.future;
        return [];
      },
    )..setActive(false);
    addTearDown(() {
      sync.dispose();
      account.dispose();
    });
    await sync.remove('wss://a-private.example');
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
    expect(find.textContaining('a-private.example'), findsOneWidget);
    await account.clearLocalSession();
    backend.subject = 'B';
    await _signIn(account);
    await sync.remove('wss://b-private.example');
    gate = Completer<void>();
    await tester.pump();
    await tester.pump();
    expect(find.textContaining('a-private.example'), findsNothing);
    gate.complete();
    gate = null;
    await tester.pumpAndSettle();
    expect(find.textContaining('b-private.example'), findsOneWidget);
  });

  testWidgets(
    'denied removal recovery opens browser and same-account exchange resumes it',
    (tester) async {
      final backend = _Backend();
      final account = backend.client();
      await _signIn(account);
      final storage = CommunityStorage(secure: FakeSecureStorage());
      final sync = _sync(account, storage);
      final browser = DiscoveryBrowser();
      final store = AccountIdentityStore(subject: 'A');
      final flow = HostedFlow(
        account: account,
        browser: browser,
        store: store,
        identity: HostedIdentityService(account: account, store: store),
        openWeb: (_) async => true,
        enterCommunity: (_, _, _, _) async {},
      );
      await flow.initialize();
      backend.denied = true;
      await sync.remove('wss://denied.example');
      await sync.synchronize();
      expect(sync.failure, AccountFailure.denied);
      await tester.pumpWidget(
        WidgetHelpers.testable(
          enableAccountCommunitySync: true,
          overrides: [
            builderlabAccountProvider.overrideWithValue(account),
            knownCommunitySyncProvider.overrideWithValue(sync),
            hostedFlowProvider.overrideWithValue(flow),
          ],
          child: const AccountCommunitySyncStatus(),
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Sign in again'));
      await tester.pumpAndSettle();
      expect(browser.startedIntent, isNotNull);
      backend.denied = false;
      browser.callbacks.add(
        BrowserAuthCode('fresh', 'a' * 43, browser.startedIntent!),
      );
      await tester.pumpAndSettle();
      await sync.synchronize();
      expect(await sync.pendingRemovals('A'), isEmpty);
      expect(backend.edits, [
        (subject: 'A', op: 'remove', url: 'wss://denied.example'),
      ]);
      await tester.pumpWidget(const SizedBox.shrink());
      flow.dispose();
      await browser.dispose();
      sync.dispose();
      account.dispose();
    },
  );
}
