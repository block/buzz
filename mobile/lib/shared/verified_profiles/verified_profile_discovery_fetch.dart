import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;

import '../relay/nip11_document.dart';
import '../relay/relay.dart';
import 'verified_profile.dart';

/// Fetches and parses the community's NIP-11 document. Throws on any failure
/// or malformed response; a thrown error never changes the last known state.
typedef VerifiedProfileDiscoveryFetch =
    Future<VerifiedProfileDiscovery> Function();

final verifiedProfileHttpClientProvider = Provider<http.Client>((ref) {
  final client = http.Client();
  ref.onDispose(client.close);
  return client;
});

/// Reads and parses the NIP-11 document at [uri]; see [fetchNip11Document].
Future<VerifiedProfileDiscovery> fetchVerifiedProfileDiscovery(
  http.Client client,
  Uri uri, {
  Duration deadline = nip11Deadline,
}) async => parseVerifiedProfileDiscovery(
  await fetchNip11Document(client, uri, deadline: deadline),
);

final verifiedProfileDiscoveryFetchProvider =
    Provider<VerifiedProfileDiscoveryFetch>((ref) {
      final client = ref.watch(verifiedProfileHttpClientProvider);
      final uri = Uri.parse(
        ref.watch(relayConfigProvider.select((config) => config.baseUrl)),
      );
      return () => fetchVerifiedProfileDiscovery(client, uri);
    });
