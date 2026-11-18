import 'package:http/http.dart' as http;

import '../relay/nip11_document.dart';

/// Whether the relay at [origin] requires NIP-FI admission evidence, read from
/// its NIP-11 document. Throws on any failure or malformed document.
typedef NipFiRequirementFetch = Future<bool> Function(Uri origin);

/// Reads [origin]'s NIP-11 document from its root, without following
/// redirects; see [fetchNip11Document].
Future<bool> fetchNipFiRequirement(
  http.Client client,
  Uri origin, {
  Duration deadline = nip11Deadline,
}) async => parseNipFiRequirement(
  await fetchNip11Document(
    client,
    Uri.parse(origin.origin).replace(path: '/'),
    deadline: deadline,
    followRedirects: false,
  ),
);

/// NIP-FI Discovery: `limitation.federated_identity` advertises support, as a
/// shadow-mode host does too; the host-specific `requires_nip_fi` says this
/// community requires evidence. A relay without the flag requires what it
/// supports. Same requirement rule as buzz-app's desktop discovery.
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
