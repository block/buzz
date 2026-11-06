import 'dart:async';
import 'dart:convert';

import 'package:http/http.dart' as http;

const _maxInfoBytes = 256 * 1024;
const _deadline = Duration(seconds: 10);

/// Whether the relay at [origin] requires NIP-FI admission evidence, read from
/// its NIP-11 document. Throws on any failure or malformed document.
typedef NipFiRequirementFetch = Future<bool> Function(Uri origin);

/// Reads [origin]'s NIP-11 document with a size cap and one deadline covering
/// headers and body. No credential goes on discovery.
Future<bool> fetchNipFiRequirement(
  http.Client client,
  Uri origin, {
  Duration deadline = _deadline,
}) async {
  final abort = Completer<void>();
  final request = http.AbortableRequest(
    'GET',
    Uri.parse(origin.origin).replace(path: '/'),
    abortTrigger: abort.future,
  )..headers['Accept'] = 'application/nostr+json';
  StreamIterator<List<int>>? body;
  Future<List<int>> read() async {
    final response = await client.send(request);
    final iterator = body = StreamIterator(response.stream);
    if (response.statusCode != 200) {
      throw http.ClientException(
        'NIP-11 status ${response.statusCode}',
        origin,
      );
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
    return parseNipFiRequirement(jsonDecode(utf8.decode(bytes)));
  } finally {
    if (!abort.isCompleted) abort.complete();
    unawaited(body?.cancel());
  }
}

/// NIP-FI Discovery: `limitation.federated_identity` advertises support, as a
/// shadow-mode host does too; the host-specific `requires_nip_fi` says this
/// community requires evidence. A relay without the flag requires what it
/// supports. Matches buzz-app's desktop discovery.
bool parseNipFiRequirement(Object? document) {
  if (document is! Map<String, dynamic>) {
    throw const FormatException('NIP-11 document is not an object');
  }
  final supports = switch (document['limitation']) {
    null => false,
    final Map<String, dynamic> limitation =>
      switch (limitation['federated_identity']) {
        null || false => false,
        true => true,
        _ => throw const FormatException('Malformed federated_identity'),
      },
    _ => throw const FormatException('Malformed NIP-11 limitation'),
  };
  return switch (document['requires_nip_fi']) {
    null => supports,
    final bool requires => requires,
    _ => throw const FormatException('Malformed requires_nip_fi'),
  };
}
