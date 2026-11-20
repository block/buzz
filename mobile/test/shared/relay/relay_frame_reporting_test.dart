import 'dart:convert';

import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';
import 'package:buzz/shared/relay/relay_socket.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../diagnostics/recording_diagnostics.dart';

// A frame the app cannot decode is reported, once. A frame it can decode is
// not, whatever it says.
void main() {
  for (final (name, frame, reported) in <(String, Object, bool)>[
    ('a truncated frame', '["EVENT","sub",{"id":"ab', true),
    ('an empty frame', '', true),
    ('text that is not JSON', '<html>502 Bad Gateway</html>', true),
    ('an object where an array belongs', '{"error":"nope"}', true),
    ('a JSON string', '"hello"', true),
    ('a binary frame', utf8.encode('["NOTICE","binary"]'), false),
    ('an empty array', '[]', false),
    ('a frame of a type the app ignores', '["NOTICE","hello"]', false),
  ]) {
    test('$name is ${reported ? '' : 'not '}reported', () async {
      final server = await loopbackRelay(first: [frame]);
      final keys = nostr.Keys.generate();
      final diagnostics = RecordingDiagnostics()..setMember(keys.public);
      final socket = RelaySocket(
        wsUrl: 'ws://127.0.0.1:${server.port}',
        nsec: keys.nsec,
        onMessage: (_) {},
        onConnected: () {},
        onDisconnected: (_) {},
      )..diagnostics = diagnostics;
      addTearDown(socket.dispose);

      await socket.connect();

      expect(socket.state, SocketState.connected);
      expect(diagnostics.operations, [
        if (reported) DiagnosticOperation.relayFrame,
      ]);
    });
  }
}
