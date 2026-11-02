import 'dart:async';
import 'dart:convert';

import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;

final relayInformationClientProvider = Provider<http.Client>((ref) {
  final client = http.Client();
  ref.onDispose(client.close);
  return client;
});

/// NIP-11 `self` identifies the relay signer; `pubkey` is only a contact key.
final relayAuthorProvider = FutureProvider.autoDispose.family<String?, String>((
  ref,
  relayUrl,
) async {
  final source = Uri.parse(relayUrl);
  final uri = source.replace(
    scheme: switch (source.scheme) {
      'wss' => 'https',
      'ws' => 'http',
      final scheme => scheme,
    },
  );
  final client = ref.watch(relayInformationClientProvider);
  final response = await client
      .send(
        http.Request('GET', uri)..headers['Accept'] = 'application/nostr+json',
      )
      .timeout(const Duration(seconds: 3));
  final chunks = StreamIterator(
    response.stream.timeout(const Duration(seconds: 3)),
  );
  final bytes = <int>[];
  try {
    if (response.statusCode != 200) return null;
    await (() async {
      while (await chunks.moveNext()) {
        if (bytes.length + chunks.current.length > 256 * 1024) {
          throw const FormatException('Relay information exceeds size limit');
        }
        bytes.addAll(chunks.current);
      }
    })().timeout(const Duration(seconds: 3));
  } finally {
    await chunks.cancel();
  }
  final info = jsonDecode(utf8.decode(bytes));
  final author = info is Map ? info['self'] : null;
  return author is String && RegExp(r'^[0-9a-f]{64}$').hasMatch(author)
      ? author
      : null;
});
