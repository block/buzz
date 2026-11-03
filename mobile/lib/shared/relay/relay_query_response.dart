import 'dart:convert';
import 'dart:io';
import 'dart:isolate';
import 'dart:typed_data';

import 'package:http/http.dart' as http;

import 'nostr_models.dart';

/// Turns a successful relay query response into events.
typedef RelayQueryDecoder =
    Future<List<NostrEvent>> Function(http.Response response);

/// Decodes a relay query response without blocking the UI isolate.
///
/// Inflating, UTF-8 decoding, JSON parsing, and event parsing all run in one
/// short-lived isolate, which hands the parsed events back by ownership
/// transfer instead of copying them. A per-response isolate keeps no state to
/// dispose and is the same shape as `parseProfileEventBatch`.
///
/// Throws the same [FormatException]s as [parseRelayQueryEvents].
Future<List<NostrEvent>> decodeRelayQueryResponse(http.Response response) =>
    _decodeInIsolate(response.bodyBytes, response.headers['content-type']);

// Takes only the bytes and content type so the isolate closure cannot capture
// the response, its headers, or any session state.
Future<List<NostrEvent>> _decodeInIsolate(
  Uint8List bytes,
  String? contentType,
) => Isolate.run(() => parseRelayQueryEvents(bytes, contentType));

/// Parses the event array a relay `/query` or `/profiles/search` answers.
///
/// Throws a [FormatException] for a body that is not JSON, is not an array,
/// or holds an element that is not a well-formed event.
List<NostrEvent> parseRelayQueryEvents(Uint8List bytes, String? contentType) {
  final decoded = jsonDecode(relayResponseText(bytes, contentType));
  if (decoded is! List) {
    throw const FormatException('relay returned malformed query response');
  }
  try {
    return [
      for (final eventJson in decoded)
        if (eventJson is Map<String, dynamic>)
          NostrEvent.fromJson(eventJson)
        else
          throw const FormatException('relay returned malformed query event'),
    ];
  } catch (error) {
    if (error is FormatException) rethrow;
    throw FormatException('relay returned malformed query event: $error');
  }
}

/// The text of a relay response body, inflated when the server sent gzip.
///
/// The query transport asks for gzip and leaves it compressed so inflating
/// can happen off the UI isolate. A body that a transport already inflated
/// passes through. JSON never starts with the gzip magic bytes, so the bytes
/// decide, not the `Content-Encoding` header a transport may leave behind.
/// The charset rules are those of [http.Response.body].
String relayResponseText(Uint8List bytes, String? contentType) {
  final inflated = _isGzip(bytes)
      ? Uint8List.fromList(gzip.decode(bytes))
      : bytes;
  return http.Response.bytes(
    inflated,
    200,
    headers: {'content-type': ?contentType},
  ).body;
}

bool _isGzip(Uint8List bytes) =>
    bytes.length > 2 && bytes[0] == 0x1f && bytes[1] == 0x8b;
