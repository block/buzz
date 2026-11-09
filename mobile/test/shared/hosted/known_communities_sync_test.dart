import 'dart:async';
import 'dart:convert';

import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/known_communities_sync.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import '../community/community_storage_test.dart';

/// A Builderlab session whose sign-ins and answers the test controls.
class _Account extends BuilderlabAccountClient {
  _Account()
    : super(httpClient: MockClient((_) async => http.Response('{}', 500)));

  final subject = ValueNotifier<String?>(null);
  final requests = <(String route, Map<String, dynamic> body)>[];
  final rows = <String>[];
  final failures = <String, List<Object>>{};
  int restores = 0;
  String? restoresAs;

  @override
  ValueListenable<String?> get subjectChanges => subject;

  @override
  BuilderlabAccount? get account => subject.value == null
      ? null
      : BuilderlabAccount(
          subject: subject.value!,
          expiresAt: DateTime.utc(2100),
        );

  @override
  Future<BuilderlabAccount?> restore() async {
    restores++;
    if (restoresAs != null) subject.value = restoresAs;
    return account;
  }

  /// A refusal is `(status, failure)`; any other object is thrown as is.
  void fail(String route, Object failure) =>
      failures.putIfAbsent(route, () => []).add(failure);

  @override
  Future<Map<String, dynamic>> post(
    String path,
    Map<String, dynamic> body, {
    String? expectedSubject,
  }) async {
    final session = account;
    if (session == null) throw AccountFailure.signedOut;
    if (expectedSubject != null && expectedSubject != session.subject) {
      throw AccountFailure.denied;
    }
    final route = path.split('/').last;
    requests.add((route, body));
    final queued = failures[route];
    if (queued != null && queued.isNotEmpty) {
      final failure = queued.removeAt(0);
      if (failure is _Gate) {
        await failure.release.future;
        return {};
      }
      if (failure is (int, AccountFailure)) {
        lastRejectedRequest = (path: path, status: failure.$1);
        throw failure.$2;
      }
      throw failure;
    }
    if (route == 'list') {
      return {
        'communities': [
          for (final url in rows) {'community_url': url},
        ],
      };
    }
    return {};
  }

  List<String> sent(String route) => [
    for (final request in requests)
      if (request.$1 == route) (request.$2['community_url'] as String?) ?? '',
  ];

  @override
  void dispose() {
    subject.dispose();
    super.dispose();
  }
}

class _Timer implements Timer {
  _Timer(this.wake);
  final void Function() wake;
  bool cancelled = false;
  @override
  void cancel() => cancelled = true;
  @override
  bool get isActive => !cancelled;
  @override
  int get tick => 0;
}

class _Fixture {
  _Fixture({
    List<String> memberships = const [],
    bool canAdmit = true,
    int maxAdmissionAttempts = 3,
  }) : memberships = List.of(memberships),
       _canAdmit = canAdmit {
    sync = KnownCommunitiesSync(
      account: account,
      outbox: outbox,
      client: KnownCommunitiesClient(account),
      memberships: () async => List.of(this.memberships),
      admit: (url, subject, isCurrent) async {
        admissions.add(url);
        final failure = admissionFailures[url];
        if (failure != null && failure.isNotEmpty) throw failure.removeAt(0);
        this.memberships.add(url);
        // Persistence records the join, as the production seam does.
        await outbox.record(url, removed: false);
      },
      canAdmit: () => _canAdmit,
      admittable: (url) => !url.contains('unsupported'),
      timer: (delay, wake) {
        final timer = _Timer(wake);
        timers.add((delay, timer));
        return timer;
      },
      maxAdmissionAttempts: maxAdmissionAttempts,
    );
  }

  final account = _Account();
  final secure = FakeSecureStorage();
  late final outbox = KnownCommunitiesOutbox(storage: secure);
  late final KnownCommunitiesSync sync;
  final List<String> memberships;
  final admissions = <String>[];
  final admissionFailures = <String, List<Object>>{};
  final timers = <(Duration, _Timer)>[];
  bool _canAdmit;
  set canAdmit(bool value) => _canAdmit = value;

  Future<KnownCommunitiesSyncState> state() => outbox.read();

  /// Lets queued storage and request turns run.
  Future<void> settle() async {
    for (var i = 0; i < 20; i++) {
      await Future<void>.delayed(Duration.zero);
    }
  }

  Future<void> fireRetry() async {
    final (_, timer) = timers.removeAt(0);
    expect(timer.cancelled, isFalse);
    timer.wake();
    await settle();
  }

  Future<void> dispose() async {
    sync.dispose();
    account.dispose();
    outbox.dispose();
  }
}

void main() {
  group('KnownCommunitiesSyncState', () {
    const a = 'wss://a.communities.example';
    const b = 'wss://b.communities.example';
    const c = 'wss://c.communities.example';

    test('keeps one latest intent per destination and nothing redundant', () {
      var state = KnownCommunitiesSyncState.empty.enqueue(a, removed: false);
      expect(state.outbox, [
        const KnownCommunityIntent(url: a, removed: false),
      ]);
      // The same word twice queues nothing new.
      expect(identical(state.enqueue(a, removed: false), state), isTrue);
      state = state.enqueue(a, removed: true);
      expect(state.outbox, [const KnownCommunityIntent(url: a, removed: true)]);
      // Known destinations are not re-added; their removal still queues.
      const known = KnownCommunitiesSyncState(known: [b]);
      expect(identical(known.enqueue(b, removed: false), known), isTrue);
      expect(known.enqueue(b, removed: true).outbox, hasLength(1));
      // Acknowledging moves the destination in or out of the known set and
      // drops the settled intent, keeping one that replaced it meanwhile.
      final added = KnownCommunitiesSyncState.empty
          .enqueue(a, removed: false)
          .acknowledge(const KnownCommunityIntent(url: a, removed: false));
      expect(added.known, [a]);
      expect(added.outbox, isEmpty);
      final replaced = added
          .enqueue(a, removed: true)
          .acknowledge(const KnownCommunityIntent(url: a, removed: false));
      expect(replaced.outbox, [
        const KnownCommunityIntent(url: a, removed: true),
      ]);
      expect(
        replaced
            .acknowledge(const KnownCommunityIntent(url: a, removed: true))
            .known,
        isEmpty,
      );
    });

    test(
      'merge adds what is listed, uploads unsynced memberships and keeps what '
      'the list omits, deferring to a pending intent',
      () {
        // Known for this account: a and b. The list now omits b (removed
        // elsewhere) and adds c; the device also saved d on its own.
        const d = 'wss://d.communities.example';
        final merged = const KnownCommunitiesSyncState(
          known: [a, b],
          account: 'me',
        ).merge(memberships: [a, b, d], listed: [a, c], account: 'me');
        expect(merged.add, [c]);
        expect(merged.state.known, containsAll([a, b, c]));
        expect(merged.state.outbox, [
          const KnownCommunityIntent(url: d, removed: false),
        ]);
        // A queued removal speaks for its destination: no add over it, and
        // the queued add of a listed destination is left to drain.
        final pending = KnownCommunitiesSyncState.empty
            .enqueue(a, removed: true)
            .enqueue(d, removed: false)
            .merge(memberships: [d], listed: [a, d], account: 'me');
        expect(pending.add, isEmpty);
        expect(pending.state.outbox, hasLength(2));
        // Another account's list knows nothing on behalf of this one.
        final other = const KnownCommunitiesSyncState(
          known: [a],
          account: 'me',
        ).merge(memberships: [a], listed: [], account: 'other');
        expect(other.state.account, 'other');
        expect(other.state.known, isEmpty);
        expect(other.state.outbox, [
          const KnownCommunityIntent(url: a, removed: false),
        ]);
      },
    );

    test('reads old or malformed records as empty, dropping bad entries', () {
      expect(KnownCommunitiesSyncState.fromJson(null).outbox, isEmpty);
      expect(KnownCommunitiesSyncState.fromJson('nope').known, isEmpty);
      final state = KnownCommunitiesSyncState.fromJson(
        jsonDecode(
          '{"account":"me","known":["wss://ok.example",7,"http://no.example"],'
          '"outbox":[{"url":"wss://x.example","removed":"yes"},'
          '{"url":"wss://y.example","removed":true},'
          '{"url":"wss://y.example","removed":false},3]}',
        ),
      );
      expect(state.account, 'me');
      expect(state.known, ['wss://ok.example']);
      expect(state.outbox, [
        const KnownCommunityIntent(url: 'wss://y.example', removed: false),
      ]);
      expect(
        KnownCommunitiesSyncState.fromJson(state.toJson()).outbox,
        state.outbox,
      );
    });

    test('addresses destinations the way the service spells them', () {
      expect(
        knownCommunityAddress('https://Team.Communities.Example:443/'),
        'wss://team.communities.example',
      );
      expect(
        knownCommunityAddress('wss://relay.example:8443'),
        'wss://relay.example:8443',
      );
      for (final rejected in [
        'ws://insecure.example',
        'wss://user@relay.example',
        'wss://relay.example/path',
        'wss://relay.example?x=1',
        'not a url',
      ]) {
        expect(knownCommunityAddress(rejected), isNull, reason: rejected);
      }
    });
  });

  group('KnownCommunitiesSync', () {
    const one = 'wss://one.communities.example';
    const two = 'wss://two.communities.example';
    const three = 'wss://three.communities.example';

    test(
      'merges the account list at sign-in: joins what the phone lacks, '
      'uploads what the account lacks, then follows joins and leaves',
      () async {
        final f = _Fixture(memberships: [one, 'wss://local.example']);
        addTearDown(f.dispose);
        f.account.rows.addAll([one, two, 'wss://unsupported.example']);
        f.sync.start();
        // Signed out: nothing is read or sent.
        await f.settle();
        expect(f.account.requests, isEmpty);
        expect(f.account.restores, 1);

        f.account.subject.value = 'me';
        await f.settle();
        expect(f.account.sent('list'), ['']);
        // The listed destination this device lacks is joined, the one it
        // cannot join on its own is left out, and the saved membership the
        // account never saw is uploaded; the listed one is not re-sent.
        expect(f.admissions, [two]);
        expect(f.account.sent('add'), ['wss://local.example']);
        final state = await f.state();
        expect(state.account, 'me');
        expect(
          state.known,
          containsAll([
            one,
            two,
            'wss://unsupported.example',
            'wss://local.example',
          ]),
        );
        expect(state.outbox, isEmpty);

        // A join and a leave on the phone each go out as the latest word, in
        // order, and the known set follows the accepted edits.
        await f.outbox.record(three, removed: false);
        await f.outbox.record(one, removed: true);
        await f.settle();
        expect(f.account.sent('add'), ['wss://local.example', three]);
        expect(f.account.sent('remove'), [one]);
        final after = await f.state();
        expect(after.known, isNot(contains(one)));
        expect(after.known, contains(three));
        expect(after.outbox, isEmpty);
        // Once merged, a trigger neither lists again nor resends.
        f.sync.kick();
        await f.settle();
        expect(f.account.sent('list'), hasLength(1));
        expect(f.account.sent('add'), hasLength(2));
      },
    );

    test(
      'retries a failed upload with the identical request at doubling delays, '
      'capped at a minute, and a trigger resets the backoff',
      () async {
        final f = _Fixture(memberships: [one]);
        addTearDown(f.dispose);
        f.account.subject.value = 'me';
        for (var i = 0; i < 8; i++) {
          f.account.fail('add', AccountFailure.unavailable);
        }
        f.sync.start();
        await f.settle();
        expect(f.account.sent('add'), [one]);
        expect((await f.state()).outbox, hasLength(1));
        final delays = <Duration>[];
        for (var i = 0; i < 7; i++) {
          delays.add(f.timers.single.$1);
          await f.fireRetry();
        }
        expect(delays, [
          for (final seconds in [1, 2, 4, 8, 16, 32, 60])
            Duration(seconds: seconds),
        ]);
        expect(f.account.sent('add'), List.filled(8, one));
        // Resuming the app runs again at once and starts the backoff over.
        final pending = f.timers.single.$2;
        f.sync.kick();
        await f.settle();
        expect(pending.cancelled, isTrue);
        expect(f.account.sent('add'), List.filled(9, one));
        expect((await f.state()).outbox, isEmpty);
        expect((await f.state()).known, [one]);
        expect(f.timers.where((entry) => !entry.$2.cancelled), isEmpty);
      },
    );

    test(
      'leaves and joins made while signed out wait in the outbox for the next '
      'sign-in, which also restores a saved session',
      () async {
        final f = _Fixture(memberships: [two]);
        addTearDown(f.dispose);
        f.sync.start();
        await f.outbox.record(one, removed: true);
        await f.outbox.record(two, removed: false);
        await f.settle();
        expect(f.account.requests, isEmpty);
        expect((await f.state()).outbox, hasLength(2));
        // A resume while signed out only checks for a saved session.
        f.account.restoresAs = 'me';
        f.sync.kick();
        await f.settle();
        expect(f.account.restores, 2);
        expect(f.account.sent('list'), hasLength(1));
        expect(f.account.sent('remove'), [one]);
        expect(f.account.sent('add'), [two]);
        expect((await f.state()).outbox, isEmpty);
      },
    );

    test(
      'refusals are not retried: a rejected intent parks until the next '
      'sign-in while others continue, and a missing route stops everything',
      () async {
        final f = _Fixture(memberships: [one, two]);
        addTearDown(f.dispose);
        f.account.subject.value = 'me';
        f.account.fail('add', (422, AccountFailure.rejected));
        f.sync.start();
        await f.settle();
        expect(f.account.sent('add'), [one, two]);
        expect(f.sync.parked, {'+$one'});
        expect(f.timers, isEmpty);
        expect((await f.state()).outbox, [
          const KnownCommunityIntent(url: one, removed: false),
        ]);
        f.sync.kick();
        await f.settle();
        expect(f.account.sent('add'), hasLength(2));
        // The next sign-in sends the parked intent again.
        f.account.subject.value = null;
        f.account.subject.value = 'me';
        await f.settle();
        expect(f.account.sent('add'), [one, two, one]);
        expect((await f.state()).outbox, isEmpty);

        // A service without the routes halts until the next sign-in.
        f.account.subject.value = null;
        f.account.fail('list', (404, AccountFailure.rejected));
        await f.outbox.record(three, removed: false);
        f.account.subject.value = 'me';
        await f.settle();
        expect(f.sync.halted, isTrue);
        f.sync.kick();
        await f.outbox.record(three, removed: true);
        await f.settle();
        expect(f.account.sent('list'), hasLength(3));
        expect(f.account.sent('add'), hasLength(3));
        expect(f.account.sent('remove'), isEmpty);
        f.account.subject.value = null;
        f.account.subject.value = 'me';
        await f.settle();
        expect(f.sync.halted, isFalse);
        expect(f.account.sent('remove'), [three]);
      },
    );

    test(
      'signing out abandons work in flight and keeps the outbox; another '
      'account starts from nothing known and is sent every membership',
      () async {
        final f = _Fixture(memberships: [one]);
        addTearDown(f.dispose);
        f.account.rows.add(one);
        f.account.subject.value = 'me';
        f.sync.start();
        await f.settle();
        expect(f.account.sent('add'), isEmpty);
        expect((await f.state()).known, [one]);

        final gate = Completer<void>();
        f.account.fail('add', _Gate(gate));
        await f.outbox.record(two, removed: false);
        await f.settle();
        expect(f.account.sent('add'), [two]);
        f.account.subject.value = null;
        gate.complete();
        await f.settle();
        // The accepted answer arrived after sign-out: not acknowledged.
        expect((await f.state()).outbox, [
          const KnownCommunityIntent(url: two, removed: false),
        ]);
        expect(f.sync.active, isFalse);

        // The other account's list is empty; it learns every membership.
        f.memberships.add(two);
        f.account.rows.clear();
        f.account.subject.value = 'other';
        await f.settle();
        expect(f.account.sent('add'), [two, two, one]);
        final state = await f.state();
        expect(state.account, 'other');
        expect(state.known, containsAll([one, two]));
      },
    );

    test('a join the identity cannot make yet waits for a trigger, the hosted '
        'flow owning the identity defers joins, and a destination that keeps '
        'failing is parked for this sign-in', () async {
      final f = _Fixture(canAdmit: false);
      addTearDown(f.dispose);
      f.account.rows.addAll([one, two]);
      f.account.subject.value = 'me';
      f.sync.start();
      await f.settle();
      expect(f.admissions, isEmpty);
      f.canAdmit = true;
      f.admissionFailures[one] = [const KnownCommunitiesNotReady()];
      f.sync.kick();
      await f.settle();
      expect(f.admissions, [one]);
      expect(f.timers, isEmpty);
      f.admissionFailures[two] = [
        AccountFailure.unavailable,
        AccountFailure.unavailable,
        AccountFailure.unavailable,
      ];
      f.sync.kick();
      await f.settle();
      expect(f.admissions, [one, one, two]);
      expect(f.memberships, [one]);
      await f.fireRetry();
      await f.fireRetry();
      expect(f.admissions, [one, one, two, two, two]);
      expect(f.sync.parked, {two});
      expect(f.timers, isEmpty);
      // The account already lists the joined destination: no upload.
      expect(f.account.sent('add'), isEmpty);
      expect((await f.state()).known, containsAll([one, two]));
    });
  });
}

/// A transport that answers once released; released after sign-out, its
/// answer must change nothing.
class _Gate implements Exception {
  const _Gate(this.release);
  final Completer<void> release;
}
