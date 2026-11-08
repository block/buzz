import 'dart:async';
import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../auth/auth_provider.dart';
import '../community/community_provider.dart';
import '../deeplink/deep_link.dart';
import 'account_identity_store.dart';
import 'builderlab_account.dart';
import 'hosted_admission.dart';
import 'hosted_communities.dart';
import 'hosted_community_connection.dart';
import 'hosted_environment.dart';
import 'hosted_relay_auth.dart';
import 'known_communities_store.dart';

/// Keeps this device's communities in step with the Builderlab account's
/// known-communities list, the rule desktop follows. While signed in, each
/// span (sign-in, restored session, or the app returning to the foreground)
/// merges the complete list: a listed community that is not saved here and has
/// no pending intent is joined silently, without activating it, and one that
/// cannot be joined now waits for the next span. Then each destination's latest
/// join or leave is sent one at a time. A failure to reach the service retries
/// at 1·2·4… s, capped at a minute; a refused upload is skipped until it is
/// replaced or the next span; a refused account or an unreadable list stops
/// sync until the next span. Signing out abandons in-flight work and keeps the
/// outbox.
class KnownCommunitiesSync {
  KnownCommunitiesSync({
    required this.subject,
    required this.store,
    required this.list,
    required this.upload,
    required this.saved,
    required this.join,
  }) {
    subject.addListener(_onSubject);
    store.addListener(kick);
    _onSubject();
  }

  /// The signed-in account subject, null while signed out.
  final ValueListenable<String?> subject;
  final KnownCommunitiesStore store;
  final Future<List<AddCommunityDeepLink>> Function() list;
  final Future<void> Function(String subject, KnownCommunityIntent intent)
  upload;

  /// Relay URLs of every saved community.
  final Future<Iterable<String>> Function() saved;
  final Future<void> Function(
    AddCommunityDeepLink community,
    String subject,
    bool Function() isCurrent,
  )
  join;

  int _span = 0;
  String? _spanSubject;
  bool _listed = false;
  bool _halted = false;
  bool _running = false;
  bool _rerun = false;
  bool _disposed = false;
  int _failures = 0;
  Timer? _timer;
  final _parked = <KnownCommunityIntent>{};

  /// Starts a new span: the list is read again and refused work retried.
  void refresh() {
    if (_disposed || _spanSubject == null) return;
    _startSpan(_spanSubject);
  }

  /// Runs now, or again once the run in flight settles.
  void kick() {
    if (_disposed || _spanSubject == null || _halted) return;
    _failures = 0;
    _timer?.cancel();
    if (_running) {
      _rerun = true;
    } else {
      unawaited(_run());
    }
  }

  void dispose() {
    _disposed = true;
    _span++;
    _timer?.cancel();
    subject.removeListener(_onSubject);
    store.removeListener(kick);
  }

  void _onSubject() {
    if (subject.value != _spanSubject) _startSpan(subject.value);
  }

  void _startSpan(String? signedIn) {
    _span++;
    _spanSubject = signedIn;
    _listed = false;
    _halted = false;
    _parked.clear();
    kick();
  }

  Future<void> _run() async {
    final account = _spanSubject;
    if (_disposed || account == null || _halted || _running) return;
    final span = _span;
    bool current() => !_disposed && span == _span && subject.value == account;
    _running = true;
    _rerun = false;
    try {
      if (!_listed) {
        final communities = await list();
        if (!current()) return;
        await store.listed(account, communities.map((item) => item.relayUrl));
        final pending = {for (final op in await store.pending()) op.url};
        final here = {
          for (final relay in await saved()) ?knownCommunityAddress(relay),
        };
        for (final community in communities) {
          if (!current()) return;
          if (here.contains(community.relayUrl) ||
              pending.contains(community.relayUrl)) {
            continue;
          }
          try {
            await join(community, account, current);
          } catch (error) {
            debugPrint('Known community join skipped until next sync: $error');
          }
        }
        if (!current()) return;
        _listed = true;
      }
      await store.joined(account, saved);
      for (;;) {
        if (!current()) return;
        final op = (await store.pending())
            .where((entry) => !_parked.contains(entry))
            .firstOrNull;
        if (op == null) break;
        try {
          await upload(account, op);
        } on AccountFailure catch (failure) {
          if (failure != AccountFailure.rejected &&
              failure != AccountFailure.invalidResponse) {
            rethrow;
          }
          debugPrint('Known community upload refused; skipped: $failure');
          _parked.add(op);
          continue;
        }
        if (!current()) return;
        await store.accepted(account, op);
      }
      _failures = 0;
      // A replaced or withdrawn intent no longer counts as refused.
      final pending = (await store.pending()).toSet();
      _parked.retainWhere(pending.contains);
    } catch (error) {
      if (!current()) return;
      if (error == AccountFailure.unavailable) {
        _timer = Timer(
          Duration(
            milliseconds: min(60000, 1000 * pow(2, _failures++)).toInt(),
          ),
          () => unawaited(_run()),
        );
      } else {
        debugPrint('Known communities sync stopped until next sign-in: $error');
        _halted = true;
      }
    } finally {
      _running = false;
      if (_rerun) unawaited(_run());
    }
  }
}

/// Runs [KnownCommunitiesSync] in a public build while it is watched.
final knownCommunitiesSyncProvider = Provider.autoDispose<void>((ref) {
  if (!HostedEnvironment.enabled) return;
  final account = ref.read(builderlabAccountProvider);
  final identities = AccountIdentityStore();
  final admission = HostedAdmission(
    relayAuth: ref.read(hostedRelayAuthProvider),
    account: account,
    store: identities,
  );
  final storage = ref.read(communityStorageProvider);
  final sync = KnownCommunitiesSync(
    subject: account.subjectChanges,
    store: ref.read(knownCommunitiesStoreProvider),
    list: () => loadKnownHostedCommunities(account),
    upload: (subject, op) async {
      final result = await account.post(
        '/v1/buzz/known-communities/${op.removed ? 'remove' : 'add'}',
        {'community_url': op.url},
        expectedSubject: subject,
      );
      if (result['error'] != null) throw AccountFailure.rejected;
    },
    saved: () async => [
      for (final item in await storage.loadAll()) item.relayUrl,
    ],
    join: (community, subject, isCurrent) async {
      final identity = await identities.forSubject(subject).read();
      // Setting up an identity here needs the person; leave it to entry.
      if (identity == null || !isCurrent()) return;
      await connectHostedCommunity(
        link: community,
        subject: subject,
        pubkey: identity.pubkey,
        isCurrent: isCurrent,
        storage: storage,
        admission: admission,
        // Saved without switching to it: the merge is silent.
        persist: (saved, current) => ref
            .read(authProvider.notifier)
            .authenticateWithCommunity(
              saved,
              shouldContinue: current,
              activate: false,
            ),
        recoverStarterChannels: (_, _) async {},
      );
    },
  );
  // A join or a leave reaches the store or the saved list.
  ref.listen(communityListProvider, (_, _) => sync.kick());
  final lifecycle = AppLifecycleListener(onResume: sync.refresh);
  // A saved session is otherwise restored only when a relay first needs it.
  if (account.account == null) {
    unawaited(account.restore().catchError((_) => null));
  }
  ref.onDispose(() {
    lifecycle.dispose();
    sync.dispose();
    admission.dispose();
  });
});
