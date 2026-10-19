import 'dart:convert';
import 'package:http/http.dart' as http;
import 'builderlab_account.dart';
import 'bounded_http_response.dart';

/// Public policy text bound to the exact version the user accepts.
class HostedJoinPolicy {
  const HostedJoinPolicy({
    required this.version,
    required this.ageRequired,
    this.terms,
    this.privacy,
  });
  final String version;
  final bool ageRequired;
  final String? terms;
  final String? privacy;
}

enum HostedInviteFailure { policyRequired, expired, exhausted, invalid }

/// Reads only a bounded JSON document, never follows redirects or logs bodies.
Future<Map<String, dynamic>> readJoinResponse(
  http.Client client,
  http.Request request,
) async {
  final http.Response response;
  final Map<String, dynamic> data;
  try {
    response = await boundedHttpResponse(client, request);
    if (response.statusCode == 429 ||
        (response.statusCode >= 500 && response.statusCode < 600)) {
      throw AccountFailure.unavailable;
    }
    data = jsonDecode(utf8.decode(response.bodyBytes)) as Map<String, dynamic>;
  } on FormatException {
    throw AccountFailure.invalidResponse;
  } on TypeError {
    throw AccountFailure.invalidResponse;
  }
  if (response.statusCode < 200 || response.statusCode >= 300) {
    throw switch (data['error']) {
      'join_policy_required' ||
      'join_policy_not_accepted' => HostedInviteFailure.policyRequired,
      'invite_expired' => HostedInviteFailure.expired,
      'invite_exhausted' => HostedInviteFailure.exhausted,
      'invite_invalid' => HostedInviteFailure.invalid,
      _ => AccountFailure.denied,
    };
  }
  return data;
}

Future<String?> freshJoinPolicyReceipt({
  required http.Client client,
  required Uri origin,
  required String code,
  required Future<bool> Function(HostedJoinPolicy) confirm,
  required void Function() checkCurrent,
}) async {
  Future<Map<String, dynamic>> send(
    String method,
    String path, [
    Map<String, dynamic>? body,
  ]) async {
    checkCurrent();
    final request = http.Request(
      method,
      origin.replace(scheme: 'https', path: path),
    )..followRedirects = false;
    if (body != null) {
      request.headers['Content-Type'] = 'application/json';
      request.body = jsonEncode(body);
    }
    final data = await readJoinResponse(client, request);
    checkCurrent();
    return data;
  }

  final data = await send('GET', '/api/join-policy');
  final raw = data['policy'];
  if (raw == null) return null;
  if (raw is! Map ||
      raw['version'] is! String ||
      (raw['version'] as String).isEmpty ||
      raw['age_attestation_required'] is! bool ||
      (raw['terms_markdown'] != null && raw['terms_markdown'] is! String) ||
      (raw['privacy_markdown'] != null && raw['privacy_markdown'] is! String)) {
    throw AccountFailure.invalidResponse;
  }
  final policy = HostedJoinPolicy(
    version: raw['version'] as String,
    ageRequired: raw['age_attestation_required'] as bool,
    terms: raw['terms_markdown'] as String?,
    privacy: raw['privacy_markdown'] as String?,
  );
  if (!await confirm(policy)) throw HostedInviteFailure.policyRequired;
  checkCurrent();
  final accepted = await send('POST', '/api/invites/accept-policy', {
    'code': code,
    'policy_version': policy.version,
    'age_confirmed': policy.ageRequired,
  });
  final receipt = accepted['receipt'];
  if (receipt is! String || receipt.isEmpty || receipt.length > 8192) {
    throw AccountFailure.invalidResponse;
  }
  return receipt;
}
