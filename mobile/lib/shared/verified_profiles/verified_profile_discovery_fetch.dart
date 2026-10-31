import 'dart:async';
import 'dart:convert';

import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;

import '../relay/relay.dart';
import 'verified_profile.dart';

const _maxInfoBytes = 256 * 1024;
const _discoveryTimeout = Duration(seconds: 10);

/// Fetches and parses the community's NIP-11 document. Throws on any failure
/// or malformed response; a thrown error never changes the last known state.
typedef VerifiedProfileDiscoveryFetch =
    Future<VerifiedProfileDiscovery> Function();

final verifiedProfileHttpClientProvider = Provider<http.Client>((ref) {
  final client = http.Client();
  ref.onDispose(client.close);
  return client;
});

/// Reads the NIP-11 document at [uri] with a size cap and timeout.
Future<VerifiedProfileDiscovery> fetchVerifiedProfileDiscovery(
  http.Client client,
  Uri uri,
) async {
  final response = await client
      .send(
        http.Request('GET', uri)..headers['Accept'] = 'application/nostr+json',
      )
      .timeout(_discoveryTimeout);
  if (response.statusCode != 200) {
    throw http.ClientException('NIP-11 status ${response.statusCode}', uri);
  }
  final bytes = <int>[];
  await response.stream.timeout(_discoveryTimeout).forEach((chunk) {
    if (bytes.length + chunk.length > _maxInfoBytes) {
      throw const FormatException('NIP-11 exceeds size limit');
    }
    bytes.addAll(chunk);
  });
  return parseVerifiedProfileDiscovery(jsonDecode(utf8.decode(bytes)));
}

final verifiedProfileDiscoveryFetchProvider =
    Provider<VerifiedProfileDiscoveryFetch>((ref) {
      final client = ref.watch(verifiedProfileHttpClientProvider);
      final uri = Uri.parse(
        ref.watch(relayConfigProvider.select((config) => config.baseUrl)),
      );
      return () => fetchVerifiedProfileDiscovery(client, uri);
    });
