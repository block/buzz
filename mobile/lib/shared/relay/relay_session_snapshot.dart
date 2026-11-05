part of 'relay_session.dart';

/// Per-request timeout for reads whose results can be megabytes: full
/// channel-membership discovery and the read-state snapshot. On a busy
/// account these run to several MB, and relay responses have been measured
/// arriving at about 200 KB/s, so the ordinary 8 s timeout cancelled them
/// on every attempt. 30 s matches buzz-app's large-read deadline; it is
/// about 6 MB at that rate, below the 8 MiB snapshot cap, so this is a
/// stopgap until those reads shrink.
const relayLargeReadTimeout = Duration(seconds: 30);

extension on RelaySessionNotifier {
  Future<List<NostrEvent>> _fetchReadStateSnapshot({
    required Duration timeout,
  }) async {
    if (_disposed || _ageRestricted) {
      throw StateError('Relay session is unavailable');
    }
    final generation = _contextGeneration;
    final config = _snapshotConfig;
    final nsec = config.nsec;
    if (nsec == null || nsec.isEmpty) {
      throw StateError('Snapshot requires a signing identity');
    }
    final viewer = nostr.Keys(nsec).public;
    void checkContext() {
      if (_disposed ||
          _ageRestricted ||
          generation != _contextGeneration ||
          _snapshotConfig.baseUrl != config.baseUrl ||
          _snapshotConfig.nsec != nsec) {
        throw StateError('Snapshot belongs to a retired session');
      }
    }

    Future<http.Response> send(
      http.Request request,
      int maxBytes, {
      required Duration timeout,
    }) async {
      final response = await _httpQueryClient.sendBounded(
        request,
        maxBytes: maxBytes,
        timeout: timeout,
        decorate: (client, active) => HostedRelayHttpClient(
          _snapshotAuth,
          config,
          inner: client,
          canSend: () =>
              active() && !_disposed && generation == _contextGeneration,
        ),
      );
      checkContext();
      if (response.statusCode < 200 || response.statusCode >= 300) {
        final text = relayResponseText(
          response.bodyBytes,
          response.headers['content-type'],
          maxInflatedBytes: maxBytes,
        );
        _activateRateLimitGateFromHttpError(text);
        throw RelayException(response.statusCode, text);
      }
      return response;
    }

    final origin = Uri.parse(config.baseUrl);
    final discovery = http.Request('GET', origin.resolve('/'))
      ..headers['Accept'] = 'application/nostr+json';
    // The query transport leaves gzip bodies compressed (see
    // RelayHttpQueryClient), and the relay's CDN gzips both responses. The
    // inflated size is capped too: the transport cap only sees the
    // compressed bytes.
    // Discovery is a small document, so a stall there fails fast even when
    // the snapshot itself is given a large-read timeout.
    const discoveryTimeout = Duration(seconds: 8);
    final discovered = await send(
      discovery,
      64 * 1024,
      timeout: timeout < discoveryTimeout ? timeout : discoveryTimeout,
    );
    final info = jsonDecode(
      relayResponseText(
        discovered.bodyBytes,
        discovered.headers['content-type'],
        maxInflatedBytes: 64 * 1024,
      ),
    );
    final descriptor = info is Map<String, dynamic>
        ? info['read_state_snapshot']
        : null;
    final community = readStateSnapshotCommunity(descriptor);
    final url = origin.resolve('/query');
    final body = utf8.encode(
      jsonEncode([
        {
          'kinds': [30078],
          'authors': [viewer],
          'read_state_snapshot': 1,
        },
      ]),
    );
    checkContext();
    final request = http.Request('POST', url)
      ..bodyBytes = body
      ..headers.addAll({
        'Content-Type': 'application/json',
        'Authorization': buildNip98AuthHeader(
          method: 'POST',
          url: url.toString(),
          bodyBytes: body,
          nsec: nsec,
        ),
      });
    final response = await send(
      request,
      readStateSnapshotMaxEnvelopeBytes,
      timeout: timeout,
    );
    // Inflating, parsing and Schnorr verification must not block Flutter's
    // UI isolate.
    final events = await _snapshotDecoder((
      bytes: response.bodyBytes,
      contentType: response.headers['content-type'],
      viewer: viewer,
      community: community,
    ));
    checkContext();
    return events;
  }
}
