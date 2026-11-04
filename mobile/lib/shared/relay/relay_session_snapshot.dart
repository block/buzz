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

    Future<String> send(http.Request request, int maxBytes) async {
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
      final text = utf8.decode(response.bodyBytes);
      if (response.statusCode < 200 || response.statusCode >= 300) {
        _activateRateLimitGateFromHttpError(text);
        throw RelayException(response.statusCode, text);
      }
      return text;
    }

    final origin = Uri.parse(config.baseUrl);
    final discovery = http.Request('GET', origin.resolve('/'))
      ..headers['Accept'] = 'application/nostr+json';
    final info = jsonDecode(await send(discovery, 64 * 1024));
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
    final text = await send(request, readStateSnapshotMaxBytes + 4096);
    // Parsing and Schnorr verification must not block Flutter's UI isolate.
    final events = await _snapshotDecoder((
      text: text,
      viewer: viewer,
      community: community,
    ));
    checkContext();
    return events;
  }
}
