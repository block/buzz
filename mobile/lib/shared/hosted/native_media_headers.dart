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
