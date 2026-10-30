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
      final transport = ref.read(savedIdentityProfileClientProvider)();
      var active = true;
      ref.onDispose(() {
        active = false;
        transport.close();
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
        final client = HostedRelayHttpClient(
          auth,
          config,
          inner: transport,
          canSend: () => active,
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
        final response = await client
            .send(request)
            .then(http.Response.fromStream)
            .timeout(const Duration(seconds: 5));
        if (!active || response.statusCode != 200) return null;
        final bytes = response.bodyBytes;
        final contentType = response.headers['content-type'];
        return _decodeProfile(bytes, contentType, pubkey);
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
