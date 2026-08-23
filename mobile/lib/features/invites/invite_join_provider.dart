import 'dart:convert';

import 'package:http/http.dart' as http;
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../../shared/auth/auth.dart';
import '../../shared/deeplink/deep_link.dart';
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

enum InviteJoinStatus {
  idle,
  confirming,
  claiming,
  reviewingPolicy,
  acceptingPolicy,
  declined,
  success,
  switchedExisting,
  error,
}

class InviteJoinPolicy {
  final String? termsMarkdown;
  final String? privacyMarkdown;
  final bool ageAttestationRequired;
  final String version;

  const InviteJoinPolicy({
    this.termsMarkdown,
    this.privacyMarkdown,
    required this.ageAttestationRequired,
    required this.version,
  });

  bool get agreementRequired =>
      termsMarkdown != null || privacyMarkdown != null;
}

class InviteJoinState {
  final InviteJoinStatus status;
  final InviteDeepLink? invite;
  final String? host;
  final String? communityName;
  final String? errorMessage;
  final bool requiresFreshInvite;
  final InviteJoinPolicy? policy;
  final bool ageConfirmed;
  final bool agreementConfirmed;

  const InviteJoinState({
    this.status = InviteJoinStatus.idle,
    this.invite,
    this.host,
    this.communityName,
    this.errorMessage,
    this.requiresFreshInvite = false,
    this.policy,
    this.ageConfirmed = false,
    this.agreementConfirmed = false,
  });

  InviteJoinState copyWith({
    InviteJoinStatus? status,
    InviteDeepLink? invite,
    String? host,
    String? communityName,
    String? errorMessage,
    bool? requiresFreshInvite,
    InviteJoinPolicy? policy,
    bool? ageConfirmed,
    bool? agreementConfirmed,
    bool clearErrorMessage = false,
    bool clearPolicy = false,
  }) => InviteJoinState(
    status: status ?? this.status,
    invite: invite ?? this.invite,
    host: host ?? this.host,
    communityName: communityName ?? this.communityName,
    errorMessage: clearErrorMessage ? null : errorMessage ?? this.errorMessage,
    requiresFreshInvite: requiresFreshInvite ?? this.requiresFreshInvite,
    policy: clearPolicy ? null : policy ?? this.policy,
    ageConfirmed: ageConfirmed ?? this.ageConfirmed,
    agreementConfirmed: agreementConfirmed ?? this.agreementConfirmed,
  );
}

class InviteJoinNotifier extends Notifier<InviteJoinState> {
  nostr.Keys? _pendingKeys;

  @override
  InviteJoinState build() => const InviteJoinState();

  Future<void> prepare(InviteDeepLink invite) async {
    _pendingKeys = null;
    validateInviteRelayUri(Uri.parse(invite.relayUrl));
    final communities = await ref.read(communityListProvider.future);
    final existing = _existingCommunity(communities, invite.relayUrl);
    if (existing != null) {
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

    try {
      final communities = await ref.read(communityListProvider.future);
      final existing = _existingCommunity(communities, invite.relayUrl);
      if (existing != null) {
        await ref
            .read(communityListProvider.notifier)
            .switchCommunity(existing.id);
        state = state.copyWith(
          status: InviteJoinStatus.switchedExisting,
          communityName: existing.name,
        );
        return;
      }

      final keys = _pendingKeys ?? ref.read(inviteKeyGeneratorProvider)();
      _pendingKeys = keys;
      await _claimAndJoin(invite, keys, invite.policyReceipt);
    } catch (error) {
      _setClaimError(error);
    }
  }

  void setAgeConfirmed(bool confirmed) {
    if (state.status != InviteJoinStatus.reviewingPolicy) return;
    state = state.copyWith(ageConfirmed: confirmed, clearErrorMessage: true);
  }

  void setAgreementConfirmed(bool confirmed) {
    if (state.status != InviteJoinStatus.reviewingPolicy) return;
    state = state.copyWith(
      agreementConfirmed: confirmed,
      clearErrorMessage: true,
    );
  }

  Future<void> acceptPolicy() async {
    final invite = state.invite;
    final policy = state.policy;
    if (invite == null ||
        policy == null ||
        state.status != InviteJoinStatus.reviewingPolicy) {
      return;
    }
    if (policy.ageAttestationRequired && !state.ageConfirmed) {
      state = state.copyWith(
        errorMessage: 'Confirm that you are at least 18 years old.',
      );
      return;
    }
    if (policy.agreementRequired && !state.agreementConfirmed) {
      state = state.copyWith(errorMessage: _policyAgreementError(policy));
      return;
    }

    state = state.copyWith(
      status: InviteJoinStatus.acceptingPolicy,
      clearErrorMessage: true,
    );
    try {
      final receipt = await _acceptJoinPolicy(invite, policy);
      final keys = _pendingKeys ?? ref.read(inviteKeyGeneratorProvider)();
      _pendingKeys = keys;
      await _claimAndJoin(invite, keys, receipt);
    } catch (error) {
      state = state.copyWith(
        status: InviteJoinStatus.reviewingPolicy,
        errorMessage: _friendlyPolicyAcceptanceError(error),
      );
    }
  }

  void declinePolicy() {
    if (state.status != InviteJoinStatus.reviewingPolicy) return;
    _pendingKeys = null;
    state = state.copyWith(
      status: InviteJoinStatus.declined,
      errorMessage:
          'You declined this community\'s join policy. You were not joined.',
    );
  }

  Future<void> _claimAndJoin(
    InviteDeepLink invite,
    nostr.Keys keys,
    String? policyReceipt,
  ) async {
    state = state.copyWith(
      status: InviteJoinStatus.claiming,
      clearErrorMessage: true,
    );
    try {
      final claim = await _claimInvite(invite, keys, policyReceipt);
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
      _pendingKeys = null;
      state = state.copyWith(
        status: InviteJoinStatus.success,
        communityName: community.name,
        clearPolicy: true,
      );
    } catch (error) {
      if (_isJoinPolicyRequired(error)) {
        await _loadJoinPolicy(invite);
        return;
      }
      _setClaimError(error);
    }
  }

  Future<Map<String, dynamic>> _claimInvite(
    InviteDeepLink invite,
    nostr.Keys keys,
    String? policyReceipt,
  ) async {
    final body = jsonEncode({
      'code': invite.code,
      'policy_receipt': ?policyReceipt,
    });
    final url = _inviteApiUrl(invite.relayUrl, '/api/invites/claim');
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
    final response = await _send(request);
    return _decodeObjectResponse(response, 'Invite claim');
  }

  Future<void> _loadJoinPolicy(InviteDeepLink invite) async {
    try {
      final request = http.Request(
        'GET',
        Uri.parse(_inviteApiUrl(invite.relayUrl, '/api/join-policy')),
      )..followRedirects = false;
      final response = await _send(request);
      final decoded = _decodeObjectResponse(response, 'Join policy');
      final rawPolicy = decoded['policy'];
      if (rawPolicy is! Map) {
        throw const FormatException(
          'Join policy response did not include a policy',
        );
      }
      final policyJson = Map<String, dynamic>.from(rawPolicy);
      final version = policyJson['version'];
      final ageAttestationRequired = policyJson['age_attestation_required'];
      if (version is! String ||
          version.isEmpty ||
          ageAttestationRequired is! bool) {
        throw const FormatException('Join policy returned malformed JSON');
      }
      state = state.copyWith(
        status: InviteJoinStatus.reviewingPolicy,
        policy: InviteJoinPolicy(
          termsMarkdown: _optionalNonEmptyString(policyJson['terms_markdown']),
          privacyMarkdown: _optionalNonEmptyString(
            policyJson['privacy_markdown'],
          ),
          ageAttestationRequired: ageAttestationRequired,
          version: version,
        ),
        ageConfirmed: false,
        agreementConfirmed: false,
        requiresFreshInvite: false,
        clearErrorMessage: true,
      );
    } catch (error) {
      state = state.copyWith(
        status: InviteJoinStatus.error,
        errorMessage: 'Could not load this community\'s join policy: $error',
        requiresFreshInvite: false,
        clearPolicy: true,
      );
    }
  }

  Future<String> _acceptJoinPolicy(
    InviteDeepLink invite,
    InviteJoinPolicy policy,
  ) async {
    final url = _inviteApiUrl(invite.relayUrl, '/api/invites/accept-policy');
    final request = http.Request('POST', Uri.parse(url))
      ..followRedirects = false
      ..headers['Content-Type'] = 'application/json'
      ..body = jsonEncode({
        'code': invite.code,
        'policy_version': policy.version,
        'age_confirmed': state.ageConfirmed,
      });
    final response = await _send(request);
    final decoded = _decodeObjectResponse(response, 'Policy acceptance');
    final receipt = decoded['receipt'];
    if (receipt is! String || receipt.isEmpty) {
      throw const FormatException(
        'Policy acceptance response did not include a receipt',
      );
    }
    return receipt;
  }

  Future<http.Response> _send(http.Request request) async {
    final streamedResponse = await ref
        .read(inviteJoinHttpClientProvider)
        .send(request);
    return http.Response.fromStream(streamedResponse);
  }

  void _setClaimError(Object error) {
    state = state.copyWith(
      status: InviteJoinStatus.error,
      errorMessage: _friendlyInviteError(error),
      requiresFreshInvite: _requiresFreshInvite(error),
    );
  }

  void reset() {
    _pendingKeys = null;
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

String _inviteApiUrl(String relayUrl, String path) {
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
    path: path,
  ).toString();
}

Map<String, dynamic> _decodeObjectResponse(
  http.Response response,
  String operation,
) {
  final decoded = jsonDecode(response.body.isEmpty ? '{}' : response.body);
  if (response.statusCode < 200 || response.statusCode >= 300) {
    final message = decoded is Map && decoded['error'] is String
        ? decoded['error'] as String
        : 'HTTP ${response.statusCode}';
    throw InviteClaimException(message);
  }
  if (decoded is! Map) {
    throw FormatException('$operation returned malformed JSON');
  }
  return Map<String, dynamic>.from(decoded);
}

String? _optionalNonEmptyString(Object? value) {
  if (value is! String || value.trim().isEmpty) return null;
  return value;
}

String _communityNameFromClaim(Map<String, dynamic> claim, String relayUrl) {
  final host = claim['host'];
  if (host is String && host.trim().isNotEmpty) return host.trim();
  return Community.nameFromUrl(relayUrl);
}

bool _requiresFreshInvite(Object error) {
  final message = error.toString();
  return message.contains('invite_exhausted');
}

bool _isJoinPolicyRequired(Object error) =>
    error.toString().contains('join_policy_required');

String _friendlyInviteError(Object error) {
  final message = error.toString();
  if (message.contains('invite_expired')) return 'This invite has expired.';
  if (message.contains('invite_exhausted')) {
    return 'This invite has reached its use limit. Ask for a new invite.';
  }
  if (message.contains('invite_invalid')) return 'This invite is not valid.';
  if (message.contains('join_policy_required')) {
    return 'This community requires you to review and accept its join policy.';
  }
  if (message.contains('SocketException') ||
      message.contains('Connection refused') ||
      message.contains('Network is unreachable') ||
      message.contains('No route to host')) {
    return 'Could not reach the relay. Check your connection and try again.';
  }
  return 'Could not join this community: $message';
}

String _friendlyPolicyAcceptanceError(Object error) {
  final message = error.toString();
  if (message.contains('join_policy_not_accepted')) {
    return 'The policy changed or your confirmation was not accepted. Review it and try again.';
  }
  return 'Could not accept this community\'s join policy: $message';
}

String _policyAgreementError(InviteJoinPolicy policy) {
  if (policy.termsMarkdown != null && policy.privacyMarkdown != null) {
    return 'Agree to the Terms of Service and Privacy Policy.';
  }
  if (policy.termsMarkdown != null) {
    return 'Agree to the Terms of Service.';
  }
  return 'Agree to the Privacy Policy.';
}
