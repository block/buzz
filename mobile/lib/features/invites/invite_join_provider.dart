import 'dart:convert';

import 'package:http/http.dart' as http;
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../../shared/auth/auth.dart';
import '../../shared/deeplink/deep_link.dart';
import '../../shared/relay/relay_provider.dart';
import '../../shared/relay/relay_session.dart';
import '../../shared/relay/relay_validation.dart';

final inviteJoinHttpClientProvider = Provider<http.Client>((ref) {
  final client = http.Client();
  ref.onDispose(client.close);
  return client;
});

final inviteKeyGeneratorProvider = Provider<InviteKeyGenerator>((ref) {
  return () => nostr.Keys.generate();
});

typedef InviteKeyGenerator = nostr.Keys Function();

const _unset = Object();

/// Ensures starter-channel memberships after an invite membership claim.
abstract interface class InviteJoinRecovery {
  /// Ensures the public starters and returns the preferred focus.
  Future<String?> ensureStarterChannels();
}

/// The active relay and signing identity a recovery instance must be bound to.
class InviteJoinRecoveryScope {
  const InviteJoinRecoveryScope({
    required this.relayHttpOrigin,
    required this.nsec,
  });

  /// Canonical HTTP(S) origin used by the active relay session.
  final String relayHttpOrigin;

  /// Signing identity that must remain active for the recovery lifetime.
  final String? nsec;
}

/// Constructs a fresh recovery operation for one scoped setup attempt.
typedef InviteJoinRecoveryFactory =
    InviteJoinRecovery Function(InviteJoinRecoveryScope scope);

/// Provides recovery construction after the active community is authenticated.
final inviteJoinRecoveryProvider = Provider<InviteJoinRecoveryFactory>((ref) {
  throw StateError(
    'inviteJoinRecoveryProvider must be configured by the app root',
  );
});

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
  final bool isStarterSetupRecovery;
  final String? focusChannelId;

  const InviteJoinState({
    this.status = InviteJoinStatus.idle,
    this.invite,
    this.host,
    this.communityName,
    this.errorMessage,
    this.requiresFreshInvite = false,
    this.isStarterSetupRecovery = false,
    this.focusChannelId,
  });

  InviteJoinState copyWith({
    InviteJoinStatus? status,
    InviteDeepLink? invite,
    String? host,
    String? communityName,
    Object? errorMessage = _unset,
    bool? requiresFreshInvite,
    bool? isStarterSetupRecovery,
    Object? focusChannelId = _unset,
  }) => InviteJoinState(
    status: status ?? this.status,
    invite: invite ?? this.invite,
    host: host ?? this.host,
    communityName: communityName ?? this.communityName,
    errorMessage: identical(errorMessage, _unset)
        ? this.errorMessage
        : errorMessage as String?,
    requiresFreshInvite: requiresFreshInvite ?? this.requiresFreshInvite,
    isStarterSetupRecovery:
        isStarterSetupRecovery ?? this.isStarterSetupRecovery,
    focusChannelId: identical(focusChannelId, _unset)
        ? this.focusChannelId
        : focusChannelId as String?,
  );
}

class InviteJoinNotifier extends Notifier<InviteJoinState> {
  Community? _pendingStarterSetupCommunity;
  Future<void>? _starterSetupInFlight;

  @override
  InviteJoinState build() => const InviteJoinState();

  Future<void> prepare(InviteDeepLink invite) async {
    validateInviteRelayUri(Uri.parse(invite.relayUrl));
    final communities = await ref.read(communityListProvider.future);
    final existing = _existingCommunity(communities, invite.relayUrl);
    if (existing != null) {
      await ref
          .read(communityListProvider.notifier)
          .switchCommunity(existing.id);
      if (existing.starterSetupIncomplete) {
        _pendingStarterSetupCommunity = existing;
        state = InviteJoinState(
          status: InviteJoinStatus.claiming,
          invite: invite,
          host: _hostFromRelay(invite.relayUrl),
          communityName: existing.name,
          isStarterSetupRecovery: true,
        );
        return;
      }
      _pendingStarterSetupCommunity = null;
      state = InviteJoinState(
        status: InviteJoinStatus.switchedExisting,
        invite: invite,
        host: _hostFromRelay(invite.relayUrl),
        communityName: existing.name,
      );
      return;
    }

    _pendingStarterSetupCommunity = null;
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
      final communities = await ref.read(communityListProvider.future);
      final existing = _existingCommunity(communities, invite.relayUrl);
      if (existing != null) {
        await ref
            .read(communityListProvider.notifier)
            .switchCommunity(existing.id);
        if (existing.starterSetupIncomplete || state.isStarterSetupRecovery) {
          _pendingStarterSetupCommunity = existing;
          await startStarterSetupRecovery();
        } else {
          state = state.copyWith(
            status: InviteJoinStatus.switchedExisting,
            communityName: existing.name,
            isStarterSetupRecovery: false,
          );
        }
        return;
      }

      if (state.isStarterSetupRecovery) {
        state = state.copyWith(
          status: InviteJoinStatus.error,
          errorMessage:
              'This community is no longer available. Re-open the invite link to try again.',
        );
        return;
      }

      final keys = ref.read(inviteKeyGeneratorProvider)();
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
        starterSetupIncomplete: true,
      );
      await ref
          .read(authProvider.notifier)
          .authenticateWithCommunity(community);
      _pendingStarterSetupCommunity = community;
      state = state.copyWith(isStarterSetupRecovery: true);
      await startStarterSetupRecovery();
    } catch (error) {
      final requiresFreshInvite = _requiresFreshInvite(error);
      state = state.copyWith(
        status: InviteJoinStatus.error,
        errorMessage: _friendlyInviteError(error),
        requiresFreshInvite: requiresFreshInvite,
      );
    }
  }

  /// Runs the pending recovery after its progress UI has been presented.
  Future<void> startStarterSetupRecovery() async {
    final community = _pendingStarterSetupCommunity;
    if (community == null ||
        state.status != InviteJoinStatus.claiming ||
        !state.isStarterSetupRecovery) {
      return;
    }
    final inFlight = _starterSetupInFlight;
    if (inFlight != null) return inFlight;

    final setup = _finishStarterSetup(community);
    _starterSetupInFlight = setup;
    try {
      await setup;
    } finally {
      if (identical(_starterSetupInFlight, setup)) {
        _starterSetupInFlight = null;
      }
    }
  }

  Future<void> _finishStarterSetup(Community community) async {
    try {
      // Authentication and community switches invalidate the scoped providers.
      // Wait for the active community projection to settle before constructing
      // the recovery, otherwise it could capture the previous relay's actions.
      final activeCommunity = await ref.read(activeCommunityProvider.future);
      if (activeCommunity?.id != community.id ||
          activeCommunity?.relayUrl != community.relayUrl ||
          activeCommunity?.nsec != community.nsec) {
        throw StateError('Active community changed before invite recovery');
      }
      final config = RelayConfig(
        baseUrl: community.relayUrl,
        nsec: community.nsec,
      );
      final focusChannelId = await ref
          .read(inviteJoinRecoveryProvider)(
            InviteJoinRecoveryScope(
              relayHttpOrigin: config.baseUrl,
              nsec: config.nsec,
            ),
          )
          .ensureStarterChannels();
      await _saveStarterSetupState(community, incomplete: false);
      state = state.copyWith(
        status: InviteJoinStatus.success,
        communityName: community.name,
        isStarterSetupRecovery: false,
        focusChannelId: focusChannelId,
      );
    } catch (error) {
      Object visibleError = error;
      try {
        await _saveStarterSetupState(community, incomplete: true);
      } catch (storageError) {
        visibleError = storageError;
      }
      state = state.copyWith(
        status: InviteJoinStatus.error,
        errorMessage: _friendlyStarterSetupError(visibleError),
        requiresFreshInvite: false,
        isStarterSetupRecovery: true,
        focusChannelId: null,
      );
    }
  }

  Future<void> _saveStarterSetupState(
    Community community, {
    required bool incomplete,
  }) {
    return ref.read(communityTransitionProvider).runExclusive(() async {
      final updated = community.copyWith(starterSetupIncomplete: incomplete);
      await ref.read(communityStorageProvider).save(updated);
      ref.invalidate(communityListProvider);
      ref.invalidate(activeCommunityProvider);
      ref.invalidate(authProvider);
    });
  }

  void reset() {
    _pendingStarterSetupCommunity = null;
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
  if (message.contains('invite_expired')) return 'This invite has expired.';
  if (message.contains('invite_exhausted')) {
    return 'This invite has reached its use limit. Ask for a new invite.';
  }
  if (message.contains('invite_invalid')) return 'This invite is not valid.';
  if (message.contains('join_policy_required')) {
    return 'This invite approval has expired. Re-open the invite link to try again.';
  }
  if (message.contains('SocketException') ||
      message.contains('Connection refused') ||
      message.contains('Network is unreachable') ||
      message.contains('No route to host')) {
    return 'Could not reach the relay. Check your connection and try again.';
  }
  return 'Could not join this community: $message';
}

String _friendlyStarterSetupError(Object error) {
  final message = error.toString();
  if (message.contains('SocketException') ||
      message.contains('Connection refused') ||
      message.contains('Network is unreachable') ||
      message.contains('No route to host')) {
    return 'Starter channels could not reach the relay. Check your connection and retry setup.';
  }
  return 'Starter channels could not be set up. Retry setup to try again.';
}
