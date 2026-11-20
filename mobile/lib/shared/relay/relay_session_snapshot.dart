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

    // Only the `/query` POST is a [RelayCall.query]; the discovery GET is not.
    Future<http.Response> send(
      http.Request request,
      int maxBytes, {
      bool query = false,
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
        throw query
            ? RelayQueryException(response.statusCode, text)
            : RelayException(response.statusCode, text);
      }
      return response;
    }

    final origin = Uri.parse(config.baseUrl);
    final discovery = http.Request('GET', origin.resolve('/'))
      ..headers['Accept'] = 'application/nostr+json';
    // The relay's CDN gzips both responses. The inflated size is capped too:
    // where the query transport leaves gzip bodies compressed (see
    // RelayHttpQueryClient), the transport cap only sees the compressed
    // bytes.
    final discovered = await send(discovery, 64 * 1024);
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
      query: true,
    );
    // Inflating, parsing and Schnorr verification must not block Flutter's
    // UI isolate.
    final List<NostrEvent> events;
    try {
      events = await _snapshotDecoder((
        bytes: response.bodyBytes,
        contentType: response.headers['content-type'],
        viewer: viewer,
        community: community,
      ));
    } catch (error, stackTrace) {
      // A retired session reports that, not whatever its bytes decoded to.
      checkContext();
      _diagnostics.reportError(
        error,
        stackTrace,
        operation: DiagnosticOperation.relayQuery,
      );
      rethrow;
    }
    checkContext();
    return events;
  }
}
