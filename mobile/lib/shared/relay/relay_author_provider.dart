import 'dart:async';
import 'dart:convert';

import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;

/// HTTP boundary for NIP-11 discovery.
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
  final bytes = <int>[];
  final done = Completer<void>();
  var ended = false;
  final subscription = response.stream.listen(
    (chunk) {
      if (done.isCompleted) return;
      if (bytes.length + chunk.length > 256 * 1024) {
        done.completeError(
          const FormatException('Relay information exceeds size limit'),
        );
      } else {
        bytes.addAll(chunk);
      }
    },
    onDone: () {
      ended = true;
      if (!done.isCompleted) done.complete();
    },
    onError: (Object error, StackTrace stack) {
      if (!done.isCompleted) done.completeError(error, stack);
    },
  );
  try {
    await done.future.timeout(const Duration(seconds: 3));
  } finally {
    if (!ended) {
      await subscription.cancel().timeout(const Duration(seconds: 1));
    }
  }
  if (response.statusCode != 200) return null;
  final info = jsonDecode(utf8.decode(bytes));
  final author = info is Map ? info['self'] : null;
  return author is String && RegExp(r'^[0-9a-f]{64}$').hasMatch(author)
      ? author
      : null;
}, retry: (_, _) => null);
