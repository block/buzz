import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:buzz/shared/http/app_http_client.dart';
import 'package:buzz/shared/relay/relay.dart';

final relayInformationClientProvider = Provider<http.Client>((ref) {
  final client = newAppHttpClient();
  ref.onDispose(client.close);
  return client;
});

/// NIP-11 `self` identifies the relay signer; `pubkey` is only a contact key.
final relayAuthorProvider = FutureProvider.autoDispose.family<String?, String>(
  (ref, relayUrl) async => _relayAuthor(
    await _fetchRelayInformation(
      ref.watch(relayInformationClientProvider),
      relayUrl,
    ),
  ),
);

/// What a send needs from a relay's NIP-11 document: its signer and the
/// extensions it advertises.
@immutable
class RelayInformation {
  const RelayInformation({required this.author, required this.extensions});

  /// The relay signer (`self`), or null when none is valid.
  final String? author;
  final Set<String> extensions;

  /// Whether a DM add can bring agents in place (`buzz-dm-agents`).
  bool get addsAgentsToDm => extensions.contains('buzz-dm-agents');
}

/// [relayUrl]'s NIP-11 document, read once per connection so a send can
/// consult it without waiting. Start the read ahead of the send; a send reads
/// only what has loaded, and treats nothing loaded, or a failed read, as
/// nothing advertised. A failed read is not retried until the next reconnect.
final relayInformationProvider =
    FutureProvider.family<RelayInformation?, String>((ref, relayUrl) async {
      ref.listen(relaySessionProvider, (previous, next) {
        if (next.status == SessionStatus.connected &&
            previous?.status != SessionStatus.connected) {
          ref.invalidateSelf();
        }
      });
      final info = await _fetchRelayInformation(
        ref.watch(relayInformationClientProvider),
        relayUrl,
      );
      if (info is! Map) return null;
      final advertised = info['supported_extensions'];
      return RelayInformation(
        author: _relayAuthor(info),
        extensions: {
          if (advertised is List)
            for (final extension in advertised)
              if (extension is String) extension,
        },
      );
    }, retry: (_, _) => null);

String? _relayAuthor(Object? info) {
  final author = info is Map ? info['self'] : null;
  return author is String && RegExp(r'^[0-9a-f]{64}$').hasMatch(author)
      ? author
      : null;
}

/// The decoded NIP-11 document at [relayUrl], or null on a non-200 status.
Future<Object?> _fetchRelayInformation(
  http.Client client,
  String relayUrl,
) async {
  final source = Uri.parse(relayUrl);
  final uri = source.replace(
    scheme: switch (source.scheme) {
      'wss' => 'https',
      'ws' => 'http',
      final scheme => scheme,
    },
  );
  // Completed on every exit, so a dropped, refused or timed-out response
  // stops downloading on a shared session instead of staying in flight.
  final abort = Completer<void>();
  final bytes = <int>[];
  try {
    final request = http.AbortableRequest(
      'GET',
      uri,
      abortTrigger: abort.future,
    )..headers['Accept'] = 'application/nostr+json';
    final response = await client
        .send(request)
        .timeout(
          const Duration(seconds: 3),
          onTimeout: () {
            noteHttpDeadline(request);
            throw TimeoutException(
              'Future not completed',
              const Duration(seconds: 3),
            );
          },
        );
    if (response.statusCode != 200) return null;
    final chunks = StreamIterator(
      response.stream.timeout(const Duration(seconds: 3)),
    );
    try {
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
  } finally {
    if (!abort.isCompleted) abort.complete();
  }
  return jsonDecode(utf8.decode(bytes));
}
