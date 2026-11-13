part of 'relay_session.dart';

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
      http.Request Function() buildRequest,
      int maxBytes,
    ) async {
      // Honor a relay pause, as every socket send does: a background retry
      // inside it would spend the quota user actions need. The request is
      // built after the wait so its NIP-98 signature is fresh.
      if (_rateLimitGate.isActive) {
        await _rateLimitGate.wait();
        checkContext();
      }
      final response = await _httpQueryClient.sendBounded(
        buildRequest(),
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
    // The relay's CDN gzips both responses. The inflated size is capped too:
    // where the query transport leaves gzip bodies compressed (see
    // RelayHttpQueryClient), the transport cap only sees the compressed
    // bytes.
    final discovered = await send(
      () =>
          http.Request('GET', origin.resolve('/'))
            ..headers['Accept'] = 'application/nostr+json',
      64 * 1024,
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
    final response = await send(
      () => http.Request('POST', url)
        ..bodyBytes = body
        ..headers.addAll({
          'Content-Type': 'application/json',
          'Authorization': buildNip98AuthHeader(
            method: 'POST',
            url: url.toString(),
            bodyBytes: body,
            nsec: nsec,
          ),
        }),
      readStateSnapshotMaxEnvelopeBytes,
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
