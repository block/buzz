import 'dart:async';
import 'dart:convert';
import 'dart:isolate';
import 'dart:typed_data';

import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;

import '../crypto/event_signing.dart';
import '../profile/profile_event_parser.dart';
import '../profile/user_profile.dart';
import '../../features/age_gate/age_signal_provider.dart';
import '../relay/relay_query_response.dart';
import '../relay/relay_session.dart';
import 'device_identity_source.dart';
import 'hosted_relay_auth.dart';

final savedIdentityProfileClientProvider = Provider<http.Client Function()>(
  (ref) => http.Client.new,
);

/// Fetches the saved identity's own profile without changing the active session.
final savedIdentityProfileProvider = FutureProvider.autoDispose
    .family<UserProfile?, String>((ref, pubkey) async {
      if (ref.watch(ageSignalProvider) == AgeSignalState.restricted) {
        return null;
      }
      final auth = ref.read(hostedRelayAuthProvider);
      final createTransport = ref.read(savedIdentityProfileClientProvider);
      http.Client? transport;
      var active = true;
      ref.onDispose(() {
        active = false;
        transport?.close();
      });
      return DeviceIdentitySource().loadProfile(pubkey, (config) async {
        if (!active) return null;
        final uri = Uri.parse(config.baseUrl).resolve('/query');
        final body = utf8.encode(
          jsonEncode([
            {
              'kinds': [0],
              'authors': [pubkey],
              'limit': 1,
            },
          ]),
        );
        final attemptTransport = createTransport();
        transport = attemptTransport;
        var requesting = true;
        StreamIterator<List<int>>? chunks;
        final client = HostedRelayHttpClient(
          auth,
          config,
          inner: attemptTransport,
          canSend: () => active && requesting,
        );
        final request = http.Request('POST', uri)
          ..followRedirects = false
          ..bodyBytes = body
          ..headers.addAll({
            'Content-Type': 'application/json',
            'Authorization': buildNip98AuthHeader(
              method: 'POST',
              url: uri.toString(),
              bodyBytes: body,
              nsec: config.nsec,
              pubkey: pubkey,
            ),
          });
        try {
          return await (() async {
            final response = await client.send(request);
            if (!active || !requesting) return null;
            chunks = StreamIterator(response.stream);
            // One kind-0 event should fit comfortably within 256 KiB.
            const maxBytes = 256 * 1024;
            if (response.statusCode != 200 ||
                (response.contentLength ?? 0) > maxBytes) {
              return null;
            }
            final bytes = BytesBuilder(copy: false);
            while (await chunks!.moveNext()) {
              if (!active || !requesting) return null;
              final chunk = chunks!.current;
              if (chunk.length > maxBytes - bytes.length) return null;
              bytes.add(chunk);
            }
            return _decodeProfile(
              bytes.takeBytes(),
              response.headers['content-type'],
              pubkey,
            );
          })().timeout(const Duration(seconds: 5));
        } finally {
          // Retire this attempt before another relay can be tried. A late auth
          // lookup must not send through the closed transport after timeout.
          requesting = false;
          if (chunks != null) unawaited(chunks!.cancel());
          client.close();
          transport = null;
        }
      });
    });

Future<UserProfile?> _decodeProfile(
  Uint8List bytes,
  String? contentType,
  String pubkey,
) => Isolate.run(() {
  final events = parseRelayQueryEvents(bytes, contentType);
  for (final event in events) {
    if (event.kind != 0 || event.pubkey != pubkey) continue;
    verifiedNostrEventFromJson(jsonEncode(event.toJson()));
    return parseProfileEvent(event).profile;
  }
  return null;
});
