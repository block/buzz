part of 'app_http_client.dart';

/// Where the shared HTTP client counts its requests.
///
/// The client is created outside Riverpod, so [runWithDiagnostics] sets this
/// once at startup, to the same reporter the app's root scope uses. Requests
/// are then attributed to whichever member that reporter currently names. It
/// is read when a request finishes, not when its client is created, so
/// clients made earlier count too. A build with no reporting destination
/// leaves the do-nothing reporter in place.
DiagnosticReporter appHttpDiagnostics = const NoopDiagnosticReporter();

/// Counts every request that goes through [_inner]: its backend (the host),
/// endpoint (from a fixed list), outcome, kind of failure and how long it took
/// to get response headers. A response whose body fails later is still the
/// success it was at its headers.
///
/// Counting happens as the request finishes and never changes its result.
class _CountedClient extends http.BaseClient {
  _CountedClient(this._inner, this._traffic);

  final http.Client _inner;
  final HttpTraffic _traffic;
  var _closed = false;

  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) async {
    final stopwatch = Stopwatch()..start();
    final http.StreamedResponse response;
    try {
      response = await _inner.send(request);
    } catch (error) {
      // Closing a client aborts its requests: the app cancelled them.
      if (!_closed) {
        _count(
          request,
          stopwatch,
          // An abort before any response is a deadline or stall the caller
          // gave up on.
          failure: error is http.RequestAbortedException
              ? NetworkFailure.timeout
              : classifyTransportError(error) ?? NetworkFailure.other,
        );
      }
      rethrow;
    }
    _count(request, stopwatch, failure: httpStatusFailure(response.statusCode));
    return response;
  }

  void _count(
    http.BaseRequest request,
    Stopwatch stopwatch, {
    required NetworkFailure? failure,
  }) {
    final host = request.url.host;
    if (host.isEmpty) return;
    appHttpDiagnostics.noteNetworkEvent(
      NetworkEvent(
        backend: host,
        endpoint: httpEndpoint(request.method, request.url, _traffic),
        outcome: failure == null
            ? NetworkOutcome.success
            : NetworkOutcome.failure,
        failure: failure,
        duration: stopwatch.elapsed,
      ),
      // Successes are the bulk of requests and would push the trail's useful
      // entries out of the SDK's limited buffer.
      breadcrumb: failure != null,
    );
  }

  @override
  void close() {
    _closed = true;
    _inner.close();
  }
}

/// Wraps [inner] so its requests are counted, for transports that do not come
/// from [newAppHttpClient].
http.Client countedAppHttpClient(
  http.Client inner, [
  HttpTraffic traffic = HttpTraffic.api,
]) => _CountedClient(inner, traffic);

/// The failure an HTTP [status] is, or null when it is a success.
///
/// A 4xx the app does not tell apart (a bad request, a missing resource) is a
/// [NetworkFailure.clientError]: the server answered, so it is not a transport
/// problem, but it is not a success either.
@visibleForTesting
NetworkFailure? httpStatusFailure(int status) => switch (status) {
  < 400 => null,
  401 || 403 => NetworkFailure.authRefused,
  408 => NetworkFailure.timeout,
  429 => NetworkFailure.rateLimited,
  >= 500 => NetworkFailure.serverError,
  _ => NetworkFailure.clientError,
};

/// The fixed endpoint name for a request, from its method, path and traffic
/// class. Never the path itself.
@visibleForTesting
NetworkEndpoint httpEndpoint(String method, Uri url, HttpTraffic traffic) {
  if (traffic == HttpTraffic.media) {
    return method == 'GET' || method == 'HEAD'
        ? NetworkEndpoint.mediaDownload
        : NetworkEndpoint.mediaUpload;
  }
  final path = url.path;
  if (method == 'GET' && (path.isEmpty || path == '/')) {
    return NetworkEndpoint.relayInfo;
  }
  if (method == 'POST' &&
      (path == '/query' || path == '/count' || path == '/profiles/search')) {
    return NetworkEndpoint.relayHttpQuery;
  }
  if (path.startsWith('/api/goose/')) return NetworkEndpoint.builderlabAccount;
  if (path.startsWith('/api/invites') ||
      path == '/api/join-policy' ||
      path.startsWith('/invite/')) {
    return NetworkEndpoint.communityAdmission;
  }
  return NetworkEndpoint.otherHttp;
}

/// The kind of failure a transport [error] is, or null when it is none of the
/// kinds the app tells apart.
///
/// Native sessions (`URLSession`, Cronet) are classified by
/// [transportFailure]; `dart:io` failures by type, error code and, where the
/// type is lost, message, as callers elsewhere already do.
NetworkFailure? classifyTransportError(Object? error) {
  if (error == null) return null;
  if (error is WebSocketChannelException) {
    return classifyTransportError(error.inner);
  }
  if (error is TimeoutException) return NetworkFailure.timeout;
  if (transportFailure(error) case final failure?) {
    return switch (failure) {
      TransportFailure.timedOut => NetworkFailure.timeout,
      TransportFailure.insecure => NetworkFailure.insecure,
      TransportFailure.unreachable =>
        isOfflineTransportError(error)
            ? NetworkFailure.offline
            : NetworkFailure.unreachable,
    };
  }
  if (error is TlsException) return NetworkFailure.insecure;
  if (error is WebSocketException) {
    return switch (error.httpStatusCode) {
      401 || 403 => NetworkFailure.authRefused,
      429 => NetworkFailure.rateLimited,
      final status? when status >= 500 => NetworkFailure.serverError,
      final status? when status >= 400 => NetworkFailure.relayRefused,
      _ => null,
    };
  }
  if (error is SocketException) {
    // ETIMEDOUT and ENETUNREACH, on Linux (Android) and on Darwin (iOS).
    final code = error.osError?.errorCode;
    final text = '${error.message} ${error.osError?.message}'.toLowerCase();
    if (code == 110 || code == 60 || text.contains('timed out')) {
      return NetworkFailure.timeout;
    }
    if (code == 101 || code == 51 || text.contains('network is unreachable')) {
      return NetworkFailure.offline;
    }
    return NetworkFailure.unreachable;
  }
  if (error is http.ClientException) {
    final message = error.message.toLowerCase();
    if (message.contains('connection closed') ||
        message.contains('connection reset')) {
      return NetworkFailure.disconnected;
    }
  }
  return null;
}
