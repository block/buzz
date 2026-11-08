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

final _keys = {'alice': 'aa' * 32, 'bob': 'bb' * 32};

/// The account service, this device's identities and its saved communities,
/// faked at their boundaries; the store and the sync are the production ones.
class _World {
  final subject = ValueNotifier<String?>(null);
  final store = KnownCommunitiesStore();
  KnownCommunitiesSync? sync;
  final server = <String, Set<String>>{};
  // Saved communities: relay URL to the key it was joined with.
  final local = <String, String>{};
  final joins = <String>[];
  final uploads = <String>[];
  final refuseJoin = <String>{};
  // Failures the next uploads answer with, in order.
  final failures = <AccountFailure>[];
  AccountFailure? listFailure;
  int lists = 0;

  Set<String> get account => server.putIfAbsent(subject.value!, () => {});

  void start() {
    sync = KnownCommunitiesSync(
      subject: subject,
      store: store,
      identity: (who) async => _keys[who],
      list: () async {
        lists++;
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
      saved: () async => [
        for (final MapEntry(:key, :value) in local.entries)
          (relayUrl: key, pubkey: value),
      ],
      join: (community, who, pubkey, isCurrent) async {
        if (refuseJoin.contains(community.relayUrl)) {
          throw AccountFailure.denied;
        }
        joins.add(community.relayUrl);
        local[community.relayUrl] = pubkey;
      },
    );
  }

  Future<void> leave(String url) async {
    final key = local.remove(url);
    await store.left(url, key);
  }
}

void main() {
  setUp(() => SharedPreferences.setMockInitialValues({}));

  test('merges the account list silently and uploads joins and leaves', () {
    fakeAsync((async) {
      final w = _World()..start();
      w.server['alice'] = {relay('desk'), relay('both')};
      w.local.addAll({
        relay('both'): _keys['alice']!,
        relay('phone'): _keys['alice']!,
        'wss://relay.example.com': _keys['alice']!,
        // Joined with another key: never alice's membership.
        relay('stranger'): 'cc' * 32,
      });

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
      int removals() => w.uploads.where((u) => u == '-${relay('both')}').length;
      expect(removals(), 1);
      async.elapse(const Duration(milliseconds: 999));
      expect(removals(), 1);
      async.elapse(const Duration(milliseconds: 1));
      expect(removals(), 2);
      async.elapse(const Duration(seconds: 2));
      expect(removals(), 3);
      expect(w.server['alice'], {relay('desk')});

      // A quick return to the foreground only sends the outbox; the list is
      // read again once it is 15 minutes old.
      w.server['alice']!.remove(relay('desk'));
      w.server['alice']!.add(relay('later'));
      w.sync!.resumed();
      async.flushMicrotasks();
      expect(w.lists, 1);
      expect(w.local.keys, isNot(contains(relay('later'))));
      async.elapse(KnownCommunitiesSync.relistAfter);
      final before = w.uploads.length;
      w.sync!.resumed();
      async.flushMicrotasks();
      expect(w.lists, 2);
      expect(w.joins.last, relay('later'));
      // Another device removed `desk`. The phone keeps it and, because the
      // account once held it, does not upload it again.
      expect(w.local.keys, contains(relay('desk')));
      expect(w.uploads, hasLength(before));

      // A refused upload is skipped, not retried, until the next span.
      w.failures.add(AccountFailure.rejected);
      w.local[relay('new')] = _keys['alice']!;
      w.sync!.kick();
      async.flushMicrotasks();
      expect(w.uploads.last, '+${relay('new')}');
      final refused = w.uploads.length;
      w.sync!.kick();
      async.elapse(KnownCommunitiesSync.relistAfter);
      expect(w.uploads, hasLength(refused));
      w.sync!.resumed();
      async.flushMicrotasks();
      expect(w.uploads, hasLength(refused + 1));
      expect(w.server['alice'], contains(relay('new')));

      // Another account on this phone never inherits alice's memberships,
      // and alice's list is not joined with bob's key.
      w.joins.clear();
      w.subject.value = null;
      async.flushMicrotasks();
      w.leave(relay('new'));
      w.subject.value = 'bob';
      async.flushMicrotasks();
      expect(w.joins, isEmpty);
      expect(w.server['bob'] ?? {}, isEmpty);
      expect(w.server['alice'], contains(relay('new')));
      // Alice's leave waits for alice.
      w.subject.value = 'alice';
      async.flushMicrotasks();
      expect(w.server['alice'], isNot(contains(relay('new'))));
      w.sync!.dispose();
    });
  });

  test('a leave queued while signed out survives restart and is sent', () {
    fakeAsync((async) {
      final first = _World()..start();
      first.server['alice'] = {relay('team')};
      first.local[relay('team')] = _keys['alice']!;
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
      async.elapse(KnownCommunitiesSync.relistAfter);
      w.sync!.resumed();
      async.flushMicrotasks();
      w.local[relay('later')] = _keys['alice']!;
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
