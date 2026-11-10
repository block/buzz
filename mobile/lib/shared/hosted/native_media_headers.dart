import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../relay/relay.dart';
import 'hosted_relay_auth.dart';

/// Headers for a relay media read that does not go through
/// [HostedRelayHttpClient], such as a native player or image loader that takes
/// headers up front.
///
/// [MediaGetAuthService.headersFor] alone is not enough on a hosted relay:
/// media reads also need the relay assertion, which only the hosted client adds.
/// The assertion is scoped to the relay origin and comes from the live
/// session's cache, so building headers never blocks.
final nativeMediaHeadersProvider =
    Provider<Map<String, String> Function(String)>((ref) {
      final media = ref.watch(mediaGetAuthServiceProvider);
      final hosted = ref.watch(hostedRelayAuthProvider);
      final config = ref.watch(relayConfigProvider);
      return (url) {
        final uri = Uri.tryParse(url);
        return {
          ...media.headersFor(url),
          if (uri != null) ...hosted.cachedHeadersForUrl(config, uri),
        };
      };
    });

/// [nativeMediaHeadersProvider] for a caller that can wait, such as a player
/// about to fetch. It asks for a fresh assertion when the cached one has
/// expired, which the synchronous form cannot do while the socket stays up.
/// If the assertion fails, the read goes out with whatever is cached and the
/// relay decides.
final freshNativeMediaHeadersProvider =
    Provider<Future<Map<String, String>> Function(String)>((ref) {
      final media = ref.watch(mediaGetAuthServiceProvider);
      final hosted = ref.watch(hostedRelayAuthProvider);
      final config = ref.watch(relayConfigProvider);
      return (url) async {
        final uri = Uri.tryParse(url);
        var assertion = const <String, String>{};
        if (uri != null) {
          try {
            assertion = await hosted.headersForUrl(config, uri);
          } on Object catch (error) {
            debugPrint('Native media assertion failed: $error');
            assertion = hosted.cachedHeadersForUrl(config, uri);
          }
        }
        // Minted after the wait, so a slow refresh cannot age the media proof.
        return {...media.headersFor(url), ...assertion};
      };
    });
