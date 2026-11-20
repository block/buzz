import 'dart:async';
import 'dart:convert';

import 'package:http/http.dart' as http;

const _maxDocumentBytes = 256 * 1024;

/// How long a NIP-11 read may take, headers and body together.
const nip11Deadline = Duration(seconds: 10);

/// Reads and decodes the NIP-11 document at [uri] with a size cap. One
/// wall-clock deadline covers headers and the whole body, so a relay trickling
/// bytes cannot keep the read in flight forever. No credential goes on the
/// request. Throws on a non-200 status, an oversized body, the deadline or
/// malformed JSON.
Future<Object?> fetchNip11Document(
  http.Client client,
  Uri uri, {
  Duration deadline = nip11Deadline,
  bool followRedirects = true,
}) async {
  final abort = Completer<void>();
  final request = http.AbortableRequest('GET', uri, abortTrigger: abort.future)
    ..followRedirects = followRedirects
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
      if (bytes.length + iterator.current.length > _maxDocumentBytes) {
        throw const FormatException('NIP-11 exceeds size limit');
      }
      bytes.addAll(iterator.current);
    }
    return bytes;
  }

  try {
    final bytes = await read().timeout(deadline);
    return jsonDecode(utf8.decode(bytes));
  } finally {
    if (!abort.isCompleted) abort.complete();
    unawaited(body?.cancel());
  }
}
