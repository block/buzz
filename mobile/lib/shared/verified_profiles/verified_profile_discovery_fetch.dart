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

/// Reads the NIP-11 document at [uri] with a size cap. One wall-clock
/// deadline covers headers and the whole body, so a relay trickling bytes
/// cannot keep discovery in flight forever.
Future<VerifiedProfileDiscovery> fetchVerifiedProfileDiscovery(
  http.Client client,
  Uri uri, {
  Duration deadline = _discoveryTimeout,
}) async {
  final abort = Completer<void>();
  final request = http.AbortableRequest('GET', uri, abortTrigger: abort.future)
    ..headers['Accept'] = 'application/nostr+json';
  StreamIterator<List<int>>? body;
  Future<List<int>> read() async {
    final response = await client.send(request);
    final iterator = body = StreamIterator(response.stream);
    if (response.statusCode != 200) {
      throw http.ClientException('NIP-11 status ${response.statusCode}', uri);
    }
    final bytes = <int>[];
    while (await iterator.moveNext()) {
      if (bytes.length + iterator.current.length > _maxInfoBytes) {
        throw const FormatException('NIP-11 exceeds size limit');
      }
      bytes.addAll(iterator.current);
    }
    return bytes;
  }

  try {
    final bytes = await read().timeout(deadline);
    return parseVerifiedProfileDiscovery(jsonDecode(utf8.decode(bytes)));
  } finally {
    if (!abort.isCompleted) abort.complete();
    unawaited(body?.cancel());
  }
}

final verifiedProfileDiscoveryFetchProvider =
    Provider<VerifiedProfileDiscoveryFetch>((ref) {
      final client = ref.watch(verifiedProfileHttpClientProvider);
      final uri = Uri.parse(
        ref.watch(relayConfigProvider.select((config) => config.baseUrl)),
      );
      return () => fetchVerifiedProfileDiscovery(client, uri);
    });
