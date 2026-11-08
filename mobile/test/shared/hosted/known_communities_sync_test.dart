import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/known_communities_store.dart';
import 'package:buzz/shared/hosted/known_communities_sync.dart';
import 'package:fake_async/fake_async.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

// Builds without Builderlab settings recognize the public communities domain.
String relay(String name) => 'wss://$name.communities.buzz.xyz';

/// The account service and this device, faked at their network and storage
/// boundaries; the store and the sync are the production ones.
class _World {
  _World() {
    store = KnownCommunitiesStore();
  }

  final subject = ValueNotifier<String?>(null);
  late KnownCommunitiesStore store;
  KnownCommunitiesSync? sync;
  final server = <String, Set<String>>{};
  final local = <String>[];
  final joins = <String>[];
  final uploads = <String>[];
  final refuseJoin = <String>{};
  // Failures the next uploads answer with, in order.
  final failures = <AccountFailure>[];
  AccountFailure? listFailure;

  Set<String> get account => server.putIfAbsent(subject.value!, () => {});

  void start() {
    sync = KnownCommunitiesSync(
      subject: subject,
      store: store,
      list: () async {
        if (listFailure != null) throw listFailure!;
        return [
          for (final url in account)
            AddCommunityDeepLink(relayUrl: url, name: url),
        ];
      },
      upload: (who, op) async {
        uploads.add('${op.removed ? '-' : '+'}${op.url}');
        if (failures.isNotEmpty) throw failures.removeAt(0);
        final set = server.putIfAbsent(who, () => {});
        op.removed ? set.remove(op.url) : set.add(op.url);
      },
      saved: () async => List.of(local),
      join: (community, who, isCurrent) async {
        if (refuseJoin.contains(community.relayUrl)) {
          throw AccountFailure.denied;
        }
        joins.add(community.relayUrl);
        local.add(community.relayUrl);
      },
    );
  }

  Future<void> leave(String url) async {
    local.remove(url);
    await store.left(url);
  }
}

void main() {
  setUp(() => SharedPreferences.setMockInitialValues({}));

  test('merges the account list silently and uploads joins and leaves', () {
    fakeAsync((async) {
      final w = _World()..start();
      w.server['alice'] = {relay('desk'), relay('both')};
      w.local.addAll([
        relay('both'),
        relay('phone'),
        'wss://relay.example.com',
      ]);

      // Signing in joins what desktop saved and uploads what only this phone
      // holds; a relay this build's account cannot enter is left alone.
      w.subject.value = 'alice';
      async.flushMicrotasks();
      expect(w.joins, [relay('desk')]);
      expect(w.uploads, ['+${relay('phone')}']);
      expect(w.server['alice'], {relay('desk'), relay('both'), relay('phone')});

      // Leaving here uploads the removal at once.
      w.leave(relay('phone'));
      async.flushMicrotasks();
      expect(w.uploads.last, '-${relay('phone')}');
      expect(w.server['alice'], {relay('desk'), relay('both')});

      // Unreachable service: the removal is retried after 1 s, then 2 s.
      w.failures.addAll([
        AccountFailure.unavailable,
        AccountFailure.unavailable,
      ]);
      w.leave(relay('both'));
      async.flushMicrotasks();
      expect(w.uploads.where((u) => u == '-${relay('both')}'), hasLength(1));
      async.elapse(const Duration(milliseconds: 999));
      expect(w.uploads.where((u) => u == '-${relay('both')}'), hasLength(1));
      async.elapse(const Duration(milliseconds: 1));
      expect(w.uploads.where((u) => u == '-${relay('both')}'), hasLength(2));
      async.elapse(const Duration(seconds: 2));
      expect(w.uploads.where((u) => u == '-${relay('both')}'), hasLength(3));
      expect(w.server['alice'], {relay('desk')});

      // Another device removes `desk`. The phone keeps it and, because the
      // account once held it, does not upload it again on the next span.
      w.server['alice']!.remove(relay('desk'));
      final before = w.uploads.length;
      w.sync!.refresh();
      async.flushMicrotasks();
      expect(w.local, contains(relay('desk')));
      expect(w.uploads, hasLength(before));

      // A refused upload is skipped, not retried, until the next span.
      w.failures.add(AccountFailure.rejected);
      w.local.add(relay('new'));
      w.sync!.kick();
      async.flushMicrotasks();
      expect(w.uploads.last, '+${relay('new')}');
      final refused = w.uploads.length;
      w.sync!.kick();
      async.elapse(const Duration(minutes: 2));
      expect(w.uploads, hasLength(refused));
      w.sync!.refresh();
      async.flushMicrotasks();
      expect(w.uploads, hasLength(refused + 1));
      expect(w.server['alice'], contains(relay('new')));

      // A different account knows nothing yet, so every membership goes up,
      // and nothing of alice's list is joined.
      w.joins.clear();
      w.subject.value = 'bob';
      async.flushMicrotasks();
      expect(w.joins, isEmpty);
      expect(w.server['bob'], {relay('desk'), relay('new')});
      w.sync!.dispose();
    });
  });

  test('a leave queued while signed out survives restart and is sent', () {
    fakeAsync((async) {
      final first = _World()..start();
      first.server['alice'] = {relay('team')};
      first.local.add(relay('team'));
      first.leave(relay('team'));
      async.flushMicrotasks();
      expect(first.uploads, isEmpty);
      first.sync!.dispose();

      // A fresh process reads the saved outbox. The pending removal speaks
      // for `team` in the merge, so it is not joined back first.
      final second = _World()..server['alice'] = {relay('team')};
      second.start();
      second.subject.value = 'alice';
      async.flushMicrotasks();
      expect(second.joins, isEmpty);
      expect(second.uploads, ['-${relay('team')}']);
      expect(second.server['alice'], isEmpty);
      second.sync!.dispose();
    });
  });

  test('a refused join waits; a refused account or list stops the span', () {
    fakeAsync((async) {
      final w = _World()..start();
      w.server['alice'] = {relay('closed'), relay('open')};
      w.refuseJoin.add(relay('closed'));
      w.subject.value = 'alice';
      async.flushMicrotasks();
      expect(w.joins, [relay('open')]);
      // A failed join is never read as a leave.
      expect(w.uploads, isEmpty);

      w.listFailure = AccountFailure.rejected;
      w.sync!.refresh();
      async.flushMicrotasks();
      w.local.add(relay('later'));
      w.sync!.kick();
      async.elapse(const Duration(minutes: 2));
      expect(w.uploads, isEmpty);

      // Signing out and back in starts over.
      w.listFailure = null;
      w.subject.value = null;
      async.flushMicrotasks();
      w.subject.value = 'alice';
      async.flushMicrotasks();
      expect(w.uploads, ['+${relay('later')}']);
      w.sync!.dispose();
    });
  });
}
