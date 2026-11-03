import 'dart:async';
import 'dart:convert';

import 'package:http/http.dart' as http;

/// Host policy, independent of whether a relay supports optional NIP-FI.
enum RelayAuthRequirement { legacy, optional, required }

/// Parses the additive host requirement; missing fields retain legacy behavior.
/// Malformed discovery is a failure, never evidence that an upgrade is needed.
RelayAuthRequirement parseRelayAuthRequirement(Object? info) {
  if (info is! Map) throw const FormatException('NIP-11 is not an object');
  if (!info.containsKey('requires_nip_fi')) return RelayAuthRequirement.legacy;
  final required = info['requires_nip_fi'];
  if (required is! bool) {
    throw const FormatException('Invalid NIP-FI requirement');
  }
  if (!required) return RelayAuthRequirement.optional;
  final limitation = info['limitation'];
  final federation = info['federated_identity'];
  final freshness = federation is Map
      ? federation['assertion_freshness']
      : null;
  if (limitation is! Map ||
      limitation['federated_identity'] != true ||
      federation is! Map ||
      federation['core'] != 'client-attached' ||
      freshness is! Map ||
      freshness['class'] != 'offline-jwt') {
    throw const FormatException('Incomplete required NIP-FI discovery');
  }
  return RelayAuthRequirement.required;
}

/// A failed discovery request is distinct from a successfully discovered policy.
class RelayAuthDiscoveryFailure implements Exception {
  const RelayAuthDiscoveryFailure(this.status);

  final int status;

  @override
  String toString() =>
      'Community authentication discovery failed (HTTP $status)';
}

/// Reads this host's public policy without credentials, redirects or caching.
/// The caller owns [client]; one deadline bounds headers and the entire body.
Future<RelayAuthRequirement> fetchRelayAuthRequirement(
  http.Client client,
  Uri uri, {
  Duration deadline = const Duration(seconds: 10),
}) async {
  if (!['http', 'https'].contains(uri.scheme) ||
      uri.host.isEmpty ||
      uri.userInfo.isNotEmpty ||
      uri.hasQuery ||
      uri.hasFragment ||
      !['', '/', '/info'].contains(uri.path)) {
    throw const FormatException('Expected a public community discovery URL');
  }
  final abort = Completer<void>();
  final request = http.AbortableRequest('GET', uri, abortTrigger: abort.future)
    ..followRedirects = false
    ..headers['Accept'] = 'application/nostr+json';
  StreamIterator<List<int>>? body;
  Future<RelayAuthRequirement> read() async {
    final response = await client.send(request);
    final iterator = body = StreamIterator(response.stream);
    if (response.statusCode != 200) {
      throw RelayAuthDiscoveryFailure(response.statusCode);
    }
    final bytes = <int>[];
    while (await iterator.moveNext()) {
      if (bytes.length + iterator.current.length > 256 * 1024) {
        throw const FormatException('NIP-11 exceeds size limit');
      }
      bytes.addAll(iterator.current);
    }
    return parseRelayAuthRequirement(jsonDecode(utf8.decode(bytes)));
  }

  try {
    return await read().timeout(deadline);
  } finally {
    if (!abort.isCompleted) abort.complete();
    unawaited(body?.cancel());
  }
}
