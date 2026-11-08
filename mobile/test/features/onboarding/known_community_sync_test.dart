import 'dart:async';

import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/hosted_communities.dart';
import 'package:buzz/shared/hosted/known_community_address.dart';
import 'package:buzz/shared/hosted/known_community_sync.dart';
import 'package:fake_async/fake_async.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../shared/community/community_storage_test.dart'
    show FakeSecureStorage;

class _Account extends BuilderlabAccountClient {
  final subject = ValueNotifier<String?>('owner');
  final calls = <({String subject, String path, String? relay})>[];
  Object? failure;
  Future<void> Function()? pause;
  List<Object> rows = [];

  @override
  ValueListenable<String?> get subjectChanges => subject;
  @override
  BuilderlabAccount? get account => subject.value == null
      ? null
      : BuilderlabAccount(
          subject: subject.value!,
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
    if (account?.subject != expectedSubject) {
      throw AccountFailure.differentAccount;
    }
    calls.add((
      subject: expectedSubject!,
      path: path,
      relay: body['community_url'] as String?,
    ));
    await pause?.call();
    if (failure case final error?) throw error;
    return path.endsWith('/list') ? {'communities': rows} : {};
  }
}

void main() {
  test(
    'backfills secure local addresses, then records additions and removals',
    () async {
      final account = _Account();
      var local = [
        'https://team.example:8443/',
        'wss://one.communities.buzz.xyz',
        'ws://insecure.example',
      ];
      final sync = KnownCommunitySync(
        account: account,
        storage: FakeSecureStorage(),
        localRelays: () async => local,
      );
      addTearDown(sync.dispose);
      await sync.synchronize();
      expect(account.calls.map((c) => c.relay), [
        'wss://team.example:8443',
        'wss://one.communities.buzz.xyz',
      ]);
      expect(account.calls.every((c) => c.path.endsWith('/add')), isTrue);
      await sync.synchronize();
      expect(account.calls, hasLength(2));
      await sync.remove('wss://team.example:8443');
      // Failed/cancelled local removal cannot delete the account entry.
      await sync.synchronize();
      expect(account.calls, hasLength(2));
      local = ['wss://one.communities.buzz.xyz'];
      await sync.synchronize();
      expect(account.calls.last.path, endsWith('/remove'));
      expect(account.calls.last.relay, 'wss://team.example:8443');
      expect(await sync.pendingRemovals('owner'), isEmpty);
    },
  );

  test(
    'offline removal survives restart and retries without another user action',
    () async {
      final account = _Account();
      final storage = FakeSecureStorage();
      var local = ['wss://team.example'];
      var sync = KnownCommunitySync(
        account: account,
        storage: storage,
        localRelays: () async => local,
      );
      await sync.synchronize();
      account.failure = AccountFailure.unavailable;
      await sync.remove(local.single);
      local = [];
      await sync.synchronize();
      expect(await sync.pendingRemovals('owner'), {'wss://team.example'});
      sync.dispose();
      account.failure = null;
      sync = KnownCommunitySync(
        account: account,
        storage: storage,
        localRelays: () async => local,
      );
      addTearDown(sync.dispose);
      await sync.synchronize();
      expect(account.calls.last.path, endsWith('/remove'));
      expect(await sync.pendingRemovals('owner'), isEmpty);
    },
  );

  test(
    'a removal queued during an add wins over the late add response',
    () async {
      final account = _Account();
      final started = Completer<void>();
      final release = Completer<void>();
      account.pause = () async {
        if (!started.isCompleted) started.complete();
        await release.future;
      };
      var local = ['wss://team.example'];
      final sync = KnownCommunitySync(
        account: account,
        storage: FakeSecureStorage(),
        localRelays: () async => local,
      );
      addTearDown(sync.dispose);
      final running = sync.synchronize();
      await started.future;
      await sync.remove(local.single);
      local = [];
      sync.trigger();
      release.complete();
      await running;
      expect(account.calls.map((c) => c.path.split('/').last), [
        'add',
        'remove',
      ]);
      expect(await sync.pendingRemovals('owner'), isEmpty);
    },
  );

  test('old account work never drains using a new account session', () async {
    final account = _Account();
    final storage = FakeSecureStorage();
    final started = Completer<void>();
    final release = Completer<void>();
    final sync = KnownCommunitySync(
      account: account,
      storage: storage,
      localRelays: () async => [],
    );
    addTearDown(sync.dispose);
    await sync.remove('wss://old.example');
    account.pause = () async {
      if (!started.isCompleted) started.complete();
      await release.future;
    };
    final running = sync.synchronize();
    await started.future;
    account.subject.value = 'another';
    release.complete();
    await running;
    expect(account.calls.map((c) => c.subject), ['owner']);
    expect(await sync.pendingRemovals('owner'), {'wss://old.example'});
    account.subject.value = 'owner';
    await sync.synchronize();
    expect(account.calls.map((c) => c.subject), ['owner', 'owner']);
    expect(await sync.pendingRemovals('owner'), isEmpty);
  });

  test(
    'refused API parks edits until sign-in instead of retrying indefinitely',
    () async {
      final account = _Account()..failure = AccountFailure.rejected;
      final sync = KnownCommunitySync(
        account: account,
        storage: FakeSecureStorage(),
        localRelays: () async => ['wss://team.example'],
      );
      addTearDown(sync.dispose);
      await sync.synchronize();
      await sync.synchronize();
      expect(account.calls, hasLength(1));
      account.failure = null;
      account.subject.value = null;
      await sync.synchronize();
      account.subject.value = 'owner';
      await sync.synchronize();
      expect(account.calls, hasLength(2));
    },
  );

  test(
    'transient retries pause in background and resume without user action',
    () {
      fakeAsync((clock) {
        final account = _Account()..failure = AccountFailure.unavailable;
        final sync = KnownCommunitySync(
          account: account,
          storage: FakeSecureStorage(),
          localRelays: () async => ['wss://team.example'],
        );
        sync.trigger();
        clock.flushMicrotasks();
        expect(account.calls, hasLength(1));
        clock.elapse(const Duration(seconds: 2));
        expect(account.calls, hasLength(2));
        sync.setActive(false);
        clock.elapse(const Duration(minutes: 1));
        expect(account.calls, hasLength(2));
        account.failure = null;
        sync.setActive(true);
        clock.flushMicrotasks();
        expect(account.calls, hasLength(3));
        clock.elapse(const Duration(minutes: 1));
        expect(account.calls, hasLength(3));
        sync.dispose();
      });
    },
  );

  test(
    'account discovery preserves supported external addresses and ports',
    () async {
      final account = _Account()
        ..rows = [
          {'community_url': 'wss://team.example:8443/'},
          {'community_url': 'wss://one.communities.buzz.xyz:443'},
          {'community_url': 'wss://one.communities.buzz.xyz/'},
          {'community_url': 'ws://insecure.example'},
          {'community_url': 'wss://user:pass@team.example'},
          {'community_url': 'wss://team.example/path'},
        ];
      final all = await loadKnownAccountCommunities(account);
      expect(all.map((c) => c.relayUrl), [
        'wss://team.example:8443',
        'wss://one.communities.buzz.xyz',
      ]);
      final hosted = await loadKnownHostedCommunities(account);
      expect(hosted.map((c) => c.relayUrl), ['wss://one.communities.buzz.xyz']);
      expect(knownCommunityAddress('wss://team.example:65536'), isNull);
      expect(knownCommunityAddress('wss://team.example:0'), isNull);
    },
  );
}
