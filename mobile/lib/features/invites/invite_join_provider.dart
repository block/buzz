import 'dart:convert';

import 'package:http/http.dart' as http;
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:url_launcher/url_launcher.dart';

import '../../shared/hosted/hosted_relay_auth.dart';
import '../../shared/hosted/community_entry_requirements.dart';
import '../../shared/auth/auth.dart';
import '../../shared/deeplink/deep_link.dart';
import '../../shared/relay/relay_session.dart';
import '../../shared/relay/relay_validation.dart';

final inviteJoinHttpClientProvider = Provider<http.Client>((ref) {
  final client = ref.watch(hostedHttpClientFactoryProvider)();
  ref.onDispose(client.close);
  return client;
});

/// Opens policy acceptance in the browser without claiming another identity.
final invitePolicyLauncherProvider = Provider<Future<bool> Function(Uri)>((
  ref,
) {
  return (uri) => launchUrl(uri, mode: LaunchMode.externalApplication);
});

final inviteKeyGeneratorProvider = Provider<InviteKeyGenerator>((ref) {
  return () => nostr.Keys.generate();
});

typedef InviteKeyGenerator = nostr.Keys Function();

const _unset = Object();

enum InviteJoinStatus {
  idle,
  confirming,
  claiming,
  success,
  switchedExisting,
  error,
}

class InviteJoinState {
  final InviteJoinStatus status;
  final InviteDeepLink? invite;
  final String? host;
  final String? communityName;
  final String? errorMessage;
  final bool requiresFreshInvite;
  final bool requiresPolicyAcceptance;

  const InviteJoinState({
    this.status = InviteJoinStatus.idle,
    this.invite,
    this.host,
    this.communityName,
    this.errorMessage,
    this.requiresFreshInvite = false,
    this.requiresPolicyAcceptance = false,
  });

  InviteJoinState copyWith({
    InviteJoinStatus? status,
    InviteDeepLink? invite,
    String? host,
    String? communityName,
    Object? errorMessage = _unset,
    bool? requiresFreshInvite,
    bool? requiresPolicyAcceptance,
  }) => InviteJoinState(
    status: status ?? this.status,
    invite: invite ?? this.invite,
    host: host ?? this.host,
    communityName: communityName ?? this.communityName,
    errorMessage: identical(errorMessage, _unset)
        ? this.errorMessage
        : errorMessage as String?,
    requiresFreshInvite: requiresFreshInvite ?? this.requiresFreshInvite,
    requiresPolicyAcceptance:
        requiresPolicyAcceptance ?? this.requiresPolicyAcceptance,
  );
}

class InviteJoinNotifier extends Notifier<InviteJoinState> {
  @override
  InviteJoinState build() => const InviteJoinState();

  Future<void> prepare(InviteDeepLink invite) async {
    validateInviteRelayUri(Uri.parse(invite.relayUrl));
    final communities = await ref.read(communityListProvider.future);
    final existing = _existingCommunity(communities, invite.relayUrl);
    if (existing != null) {
      await _retirePendingInvite(invite, existing);
      await ref
          .read(communityListProvider.notifier)
          .switchCommunity(existing.id);
      state = InviteJoinState(
        status: InviteJoinStatus.switchedExisting,
        invite: invite,
        host: _hostFromRelay(invite.relayUrl),
        communityName: existing.name,
      );
      return;
    }

    state = InviteJoinState(
      status: InviteJoinStatus.confirming,
      invite: invite,
      host: _hostFromRelay(invite.relayUrl),
      communityName: Community.nameFromUrl(invite.relayUrl),
    );
  }

  Future<void> confirmJoin() async {
    final invite = state.invite;
    if (invite == null ||
        state.requiresFreshInvite ||
        (state.status != InviteJoinStatus.confirming &&
            state.status != InviteJoinStatus.error)) {
      return;
    }

    state = state.copyWith(
      status: InviteJoinStatus.claiming,
      errorMessage: null,
      requiresFreshInvite: false,
    );
    try {
      CommunityEntryRequirements.requireSelfHosted(invite.relayUrl);
      final communities = await ref.read(communityListProvider.future);
      final existing = _existingCommunity(communities, invite.relayUrl);
      if (existing != null) {
        await _retirePendingInvite(invite, existing);
        await ref
            .read(communityListProvider.notifier)
            .switchCommunity(existing.id);
        state = state.copyWith(
          status: InviteJoinStatus.switchedExisting,
          communityName: existing.name,
        );
        return;
      }

      final storage = ref.read(communityStorageProvider);
      final nsec = await storage.pendingInviteIdentity(
        invite.relayUrl,
        invite.code,
        () => ref.read(inviteKeyGeneratorProvider)().nsec,
      );
      final decodedKey = nostr.Nip19.decode(payload: nsec);
      if (decodedKey.prefix != nostr.Nip19Prefix.nsec) {
        throw const FormatException('Pending invite identity is unreadable');
      }
      final keys = nostr.Keys(decodedKey.data);
      final body = jsonEncode({
        'code': invite.code,
        if (invite.policyReceipt != null)
          'policy_receipt': invite.policyReceipt,
      });
      final relayUri = Uri.parse(invite.relayUrl);
      validateInviteRelayUri(relayUri);
      final url = _claimUrlFromRelay(invite.relayUrl);
      final request = http.Request('POST', Uri.parse(url))
        ..followRedirects = false
        ..headers.addAll({
          'Authorization': buildNip98AuthHeader(
            method: 'POST',
            url: url,
            bodyBytes: utf8.encode(body),
            nsec: keys.nsec,
          ),
          'Content-Type': 'application/json',
        })
        ..body = body;
      final streamedResponse = await ref
          .read(inviteJoinHttpClientProvider)
          .send(request);
      final response = await http.Response.fromStream(streamedResponse);
      final decoded = jsonDecode(response.body.isEmpty ? '{}' : response.body);
      if (response.statusCode < 200 || response.statusCode >= 300) {
        final message = decoded is Map && decoded['error'] is String
            ? decoded['error'] as String
            : 'HTTP ${response.statusCode}';
        throw InviteClaimException(message);
      }
      if (decoded is! Map) {
        throw const FormatException('Invite claim returned malformed JSON');
      }
      final claim = Map<String, dynamic>.from(decoded);

      final community = Community.create(
        name: _communityNameFromClaim(claim, invite.relayUrl),
        relayUrl: invite.relayUrl,
        pubkey: keys.public,
        nsec: keys.nsec,
        sensitiveActionPolicy: SensitiveActionPolicy.disabledByUser,
      );
      await ref
          .read(authProvider.notifier)
          .authenticateWithCommunity(community);
      await _retirePendingInvite(invite, community);
      state = state.copyWith(
        status: InviteJoinStatus.success,
        communityName: community.name,
      );
    } catch (error) {
      final requiresFreshInvite = _requiresFreshInvite(error);
      state = state.copyWith(
        status: InviteJoinStatus.error,
        errorMessage: _friendlyInviteError(error),
        requiresFreshInvite: requiresFreshInvite,
        requiresPolicyAcceptance: error.toString().contains(
          'join_policy_required',
        ),
      );
    }
  }

  /// Opens the canonical invite so the user can accept the current join policy.
  Future<bool> openPolicyAcceptance() async {
    final invite = state.invite;
    if (!state.requiresPolicyAcceptance || invite == null) return false;
    final relay = Uri.parse(invite.relayUrl);
    final uri = relay.replace(
      scheme: relay.scheme == 'wss' ? 'https' : 'http',
      pathSegments: ['invite', invite.code],
    );
    try {
      if (await ref.read(invitePolicyLauncherProvider)(uri)) return true;
    } catch (_) {
      // Keep the pending identity and expose the same action for another try.
    }
    state = state.copyWith(
      errorMessage:
          'Could not open the browser. Try reviewing the join requirements again.',
    );
    return false;
  }

  Future<void> _retirePendingInvite(
    InviteDeepLink invite,
    Community community,
  ) async {
    try {
      await ref
          .read(communityStorageProvider)
          .completePendingInvite(invite.relayUrl, invite.code, community);
    } catch (_) {
      // The saved destination is authoritative. Reopening retries cleanup;
      // community removal must erase every journal copy before it succeeds.
    }
  }

  void reset() {
    state = const InviteJoinState();
  }
}

final inviteJoinProvider =
    NotifierProvider<InviteJoinNotifier, InviteJoinState>(
      InviteJoinNotifier.new,
    );

class InviteClaimException implements Exception {
  final String message;

  const InviteClaimException(this.message);

  @override
  String toString() => message;
}

Community? _existingCommunity(List<Community> communities, String relayUrl) {
  final invite = _relayOriginForComparison(relayUrl);
  for (final community in communities) {
    final current = _relayOriginForComparison(community.relayUrl);
    if (current == null) continue;
    if (current == invite) {
      return community;
    }
  }
  return null;
}

({bool secure, String host, int? port})? _relayOriginForComparison(String url) {
  final uri = Uri.tryParse(url);
  if (uri == null || uri.host.isEmpty) return null;
  final secure = switch (uri.scheme) {
    'https' || 'wss' => true,
    'http' || 'ws' => false,
    _ => null,
  };
  if (secure == null) return null;
  return (
    secure: secure,
    host: uri.host.toLowerCase(),
    port: _effectivePort(uri),
  );
}

int? _effectivePort(Uri uri) {
  if (uri.hasPort) return uri.port;
  return switch (uri.scheme) {
    'https' || 'wss' => 443,
    'http' || 'ws' => 80,
    _ => null,
  };
}

String _hostFromRelay(String relayUrl) {
  final uri = Uri.parse(relayUrl);
  if (uri.hasPort) return '${uri.host}:${uri.port}';
  return uri.host;
}

String _claimUrlFromRelay(String relayUrl) {
  final uri = Uri.parse(relayUrl);
  final scheme = switch (uri.scheme) {
    'wss' => 'https',
    'ws' => 'http',
    _ => throw FormatException('Invalid relay URL scheme: ${uri.scheme}'),
  };
  return Uri(
    scheme: scheme,
    host: uri.host,
    port: uri.hasPort ? uri.port : null,
    path: '/api/invites/claim',
  ).toString();
}

String _communityNameFromClaim(Map<String, dynamic> claim, String relayUrl) {
  final host = claim['host'];
  if (host is String && host.trim().isNotEmpty) return host.trim();
  return Community.nameFromUrl(relayUrl);
}

bool _requiresFreshInvite(Object error) {
  final message = error.toString();
  return message.contains('join_policy_required') ||
      message.contains('invite_exhausted');
}

String _friendlyInviteError(Object error) {
  final message = error.toString();
  if (message.contains('invite_restricted')) {
    return 'This account, or the account that owns it, is restricted in this community. Contact a community administrator. If the restriction is temporary, try again after it ends.';
  }
  if (message.contains('invite_owner_mismatch')) {
    return 'This account does not match its recorded owner. Use the correct account or owner identity to join this community.';
  }
  if (message.contains('invite_expired')) return 'This invite has expired.';
  if (message.contains('invite_exhausted')) {
    return 'This invite has reached its use limit. Ask for a new invite.';
  }
  if (message.contains('invite_invalid')) return 'This invite is not valid.';
  if (message.contains('join_policy_required')) {
    return 'Review this community’s join requirements in your browser, then return to Buzz.';
  }
  if (message.contains('SocketException') ||
      message.contains('Connection refused') ||
      message.contains('Network is unreachable') ||
      message.contains('No route to host')) {
    return 'Could not reach the relay. Check your connection and try again.';
  }
  return 'Could not join this community: $message';
}
