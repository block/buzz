import 'dart:convert';

import 'package:http/http.dart' as http;

import '../community/community.dart';
import '../deeplink/deep_link.dart';
import '../relay/relay_session.dart';
import '../relay/relay_socket.dart';
import 'account_identity_store.dart';
import 'builderlab_account.dart';
import 'community_entry_requirements.dart';
import 'hosted_environment.dart';
import 'hosted_communities.dart';
import 'hosted_join_policy.dart';
import 'hosted_relay_auth.dart';
import '../relay/relay_provider.dart';

/// Builds a community only after scoped backup evidence and current binding.
/// The caller persists it through the existing community transition owner.
class HostedAdmission {
  HostedAdmission({
    required this.account,
    required this.store,
    http.Client? client,
    required this.relayAuth,
    Future<bool> Function(HostedJoinPolicy)? confirmPolicy,
    Future<void> Function(Community)? probe,
  }) : _confirmPolicy = confirmPolicy ?? ((_) async => false),
       _http = client ?? http.Client(),
       _probe = probe;
  final HostedRelayAuth relayAuth;
  final BuilderlabAccountClient account;
  final AccountIdentityStore store;
  final http.Client _http;
  final Future<bool> Function(HostedJoinPolicy) _confirmPolicy;
  final Future<void> Function(Community)? _probe;

  Future<Community> prepare(
    BuzzDeepLink link,
    String subject,
    String pubkey, {
    bool Function()? isCurrent,
  }) async {
    void checkCurrent() {
      if (isCurrent != null && !isCurrent()) throw AccountFailure.unavailable;
      if (account.account?.subject != subject) {
        throw AccountFailure.differentAccount;
      }
    }

    final store = this.store.forSubject(subject);
    checkCurrent();
    final relay = link.relayUrl ?? (throw AccountFailure.invalidResponse);
    final uri = Uri.parse(relay);
    if (uri.scheme != 'wss' ||
        uri.userInfo.isNotEmpty ||
        uri.hasQuery ||
        uri.hasFragment ||
        (uri.path.isNotEmpty && uri.path != '/') ||
        !HostedEnvironment.supportsRelay(uri) ||
        (uri.hasPort && uri.port != 443)) {
      throw AccountFailure.denied;
    }
    if (account.account?.subject != subject) {
      throw AccountFailure.differentAccount;
    }
    await CommunityEntryRequirements.verifyHostedIdentity(
      account: account,
      store: store,
      subject: subject,
      pubkey: pubkey,
    );
    checkCurrent();
    if (link is InviteDeepLink) {
      try {
        final receipt = await freshJoinPolicyReceipt(
          client: _http,
          origin: uri,
          code: link.code,
          confirm: _confirmPolicy,
          checkCurrent: checkCurrent,
        );
        final body = jsonEncode({
          'code': link.code,
          'policy_receipt': ?receipt,
        });
        final url = uri.replace(scheme: 'https', path: '/api/invites/claim');
        final headers = await store.withKeys(
          (keys) async => {
            ...await relayAuth.headers(
              RelayConfig(baseUrl: relay, nsec: keys.nsec),
            ),
            'Authorization': buildNip98AuthHeader(
              method: 'POST',
              url: url.toString(),
              bodyBytes: utf8.encode(body),
              nsec: keys.nsec,
            ),
            'Content-Type': 'application/json',
          },
        );
        checkCurrent();
        final request = http.Request('POST', url)
          ..followRedirects = false
          ..headers.addAll(headers)
          ..body = body;
        await readJoinResponse(_http, request);
        checkCurrent();
      } on HostedInviteFailure catch (failure) {
        if (failure == HostedInviteFailure.policyRequired ||
            failure == HostedInviteFailure.restricted ||
            failure == HostedInviteFailure.ownerMismatch) {
          rethrow;
        }
        checkCurrent();
        // A lost claim response may outlive its code. AUTH alone also admits
        // non-members on open relays; reconcile explicit membership first.
        final List<AddCommunityDeepLink> joined;
        try {
          joined = await loadHostedCommunities(account);
        } on AccountFailure catch (discoveryFailure) {
          if (discoveryFailure == AccountFailure.rejected ||
              discoveryFailure == AccountFailure.invalidResponse) {
            throw failure;
          }
          rethrow;
        }
        checkCurrent();
        if (!joined.any(
          (entry) => Uri.parse(entry.relayUrl).host == uri.host,
        )) {
          rethrow;
        }
      }
    }
    if (account.account?.subject != subject) {
      throw AccountFailure.differentAccount;
    }
    final community = await store.withKeys((keys) async {
      if (keys.public != pubkey) throw AccountIdentityFailure.mismatch;
      return Community.create(
        name: link is AddCommunityDeepLink ? link.name : uri.host,
        relayUrl: relay,
        pubkey: pubkey,
        nsec: keys.nsec,
      );
    });
    checkCurrent();
    await (_probe ?? _authenticateRelay)(community);
    checkCurrent();
    return community;
  }

  Future<void> _authenticateRelay(Community community) async {
    final socket = RelaySocket(
      wsUrl: community.relayUrl,
      nsec: community.nsec,
      onMessage: (_) {},
      onConnected: () {},
      onDisconnected: (_) {},
    );
    try {
      socket.headers = await relayAuth.headers(
        RelayConfig(baseUrl: community.relayUrl, nsec: community.nsec),
      );
      await socket.connect().timeout(const Duration(seconds: 30));
      if (socket.state != SocketState.connected) throw AccountFailure.denied;
    } finally {
      socket.dispose();
    }
  }

  void dispose() => _http.close();
}
