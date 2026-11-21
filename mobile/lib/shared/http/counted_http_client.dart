part of 'app_http_client.dart';

/// Where the shared HTTP client counts its requests.
///
/// The client is created outside Riverpod, so [runWithDiagnostics] sets this
/// once at startup, to the same reporter the app's root scope uses. Each
/// request is attributed to the member this reporter names when the request
/// starts, and is dropped if it finishes while it names anyone else, including
/// no one. It is read per request, not when a client is created, so clients
/// made earlier count too. A build with no reporting destination leaves the
/// do-nothing reporter in place.
DiagnosticReporter appHttpDiagnostics = const NoopDiagnosticReporter();

/// Hosts whose requests are counted under their own name: the relays this
/// run has used and the build's own services. `DiagnosticMemberBinding`
/// fills it.
final appHttpFirstPartyHosts = <String>{};

/// The backend a request to any other host is counted under, so a remote
/// image's host (which can name a person) never leaves the phone, and the
/// label stays a short fixed list.
const externalHttpBackend = 'external';

/// The backend label for [host]: the host itself when it is first party,
/// otherwise [externalHttpBackend].
@visibleForTesting
String httpBackend(String host) {
  final name = host.toLowerCase();
  return appHttpFirstPartyHosts.contains(name) ? name : externalHttpBackend;
}

/// Counts every request that goes through [_inner]: its backend
/// ([httpBackend]),
/// endpoint (from a fixed list), outcome, kind of failure and how long it took
/// to get response headers.
///
/// A failure status is counted at its headers. A success is counted once its
/// body is read to the end or its reader stops early. A body that fails or is
/// cancelled, or ends, after the caller's deadline passed ([noteHttpDeadline])
/// is a timeout, and one that fails before it, other than by the app aborting it,
/// is a failure of its kind. A body that is never read is not counted. Work
/// that outlives a member change counts for nobody.
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
    // A request can outlive a sign-out or a community switch; what it saw
    // belongs to the member who started it, not to whoever is next, nor to
    // the same member signed in again.
    final changes = appHttpDiagnostics.memberChanges;
    void count(NetworkFailure? failure, Duration duration) {
      if (appHttpDiagnostics.memberChanges != changes) return;
      _count(request, duration, failure: failure);
    }

    final http.StreamedResponse response;
    try {
      response = await _inner.send(request);
    } catch (error) {
      if (_deadlinePassed[request] ?? false) {
        // Its owner gave up waiting, whether by aborting it or by closing the
        // client it was on.
        count(NetworkFailure.timeout, stopwatch.elapsed);
      } else if (!_closed && error is! http.RequestAbortedException) {
        // An abort or close with no deadline behind it is the app cancelling
        // (a view closed, an image scrolled away), not a network failure.
        count(
          classifyTransportError(error) ?? NetworkFailure.other,
          stopwatch.elapsed,
        );
      }
      rethrow;
    }
    final headers = stopwatch.elapsed;
    if (_deadlinePassed[request] ?? false) {
      // The caller gave up before the headers arrived, whatever they say.
      count(NetworkFailure.timeout, headers);
      return response;
    }
    if (httpStatusFailure(response.statusCode) case final failure?) {
      count(failure, headers);
      return response;
    }
    var settled = false;
    void settle({required bool complete, Object? error}) {
      if (settled) return;
      settled = true;
      count(switch ((complete, error)) {
        // The caller gave up first, even if the last bytes then arrived.
        _ when _deadlinePassed[request] ?? false => NetworkFailure.timeout,
        (true, _) => null,
        // A reader that stops early, or an abort or close with no deadline
        // behind it, is the app done with the body.
        (false, null) || (false, http.RequestAbortedException()) => null,
        _ when _closed => null,
        (false, final Object error) =>
          classifyTransportError(error) ?? NetworkFailure.other,
      }, headers);
    }

    return _withStream(
      response,
      _settledStream(response.stream, onEnd: settle),
    );
  }

  void _count(
    http.BaseRequest request,
    Duration duration, {
    required NetworkFailure? failure,
  }) {
    final host = request.url.host;
    if (host.isEmpty) return;
    appHttpDiagnostics.noteNetworkEvent(
      NetworkEvent(
        backend: httpBackend(host),
        endpoint: httpEndpoint(request.method, request.url, _traffic),
        outcome: failure == null
            ? NetworkOutcome.success
            : NetworkOutcome.failure,
        failure: failure,
        duration: duration,
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

/// [body] unchanged, calling [onEnd] when it ends (complete), or fails or its
/// listener cancels (not complete, with the failure if any). A failure is
/// passed through as it was.
Stream<List<int>> _settledStream(
  Stream<List<int>> body, {
  required void Function({required bool complete, Object? error}) onEnd,
}) {
  late final StreamController<List<int>> controller;
  StreamSubscription<List<int>>? subscription;
  controller = StreamController<List<int>>(
    sync: true,
    onListen: () {
      subscription = body.listen(
        controller.add,
        onError: (Object error, StackTrace stack) {
          onEnd(complete: false, error: error);
          controller.addError(error, stack);
        },
        onDone: () {
          onEnd(complete: true);
          controller.close();
        },
      );
    },
    onPause: () => subscription?.pause(),
    onResume: () => subscription?.resume(),
    onCancel: () {
      onEnd(complete: false);
      return subscription?.cancel();
    },
  );
  return controller.stream;
}

/// [response] with its body replaced by [stream], keeping everything else.
http.StreamedResponse _withStream(
  http.StreamedResponse response,
  Stream<List<int>> stream,
) => response is http.BaseResponseWithUrl
    ? _StreamedResponseWithUrl(
        stream,
        response.statusCode,
        url: (response as http.BaseResponseWithUrl).url,
        contentLength: response.contentLength,
        request: response.request,
        headers: response.headers,
        isRedirect: response.isRedirect,
        persistentConnection: response.persistentConnection,
        reasonPhrase: response.reasonPhrase,
      )
    : http.StreamedResponse(
        stream,
        response.statusCode,
        contentLength: response.contentLength,
        request: response.request,
        headers: response.headers,
        isRedirect: response.isRedirect,
        persistentConnection: response.persistentConnection,
        reasonPhrase: response.reasonPhrase,
      );

class _StreamedResponseWithUrl extends http.StreamedResponse
    implements http.BaseResponseWithUrl {
  _StreamedResponseWithUrl(
    super.stream,
    super.statusCode, {
    required this.url,
    super.contentLength,
    super.request,
    super.headers,
    super.isRedirect,
    super.persistentConnection,
    super.reasonPhrase,
  });

  @override
  final Uri url;
}

final _deadlinePassed = Expando<bool>('HTTP deadline passed');

/// Records that [request]'s deadline passed. Call it before aborting the
/// request, or closing its client, because the caller stopped waiting.
///
/// The shared client then counts the request as a timeout, whether it was
/// still waiting for headers or reading a successful response's body. A
/// response already read to the end, or a failure status counted at its
/// headers, keeps its count. An abort or close without this is the app
/// cancelling and is not counted.
void noteHttpDeadline(http.BaseRequest request) =>
    _deadlinePassed[request] = true;

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
            : isConnectionLostTransportError(error)
            ? NetworkFailure.disconnected
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
    // ECONNRESET. Dart's HTTP client throws a socket error that is also a
    // ClientException, so the reset check below never sees it.
    if (code == 104 || code == 54 || text.contains('connection reset')) {
      return NetworkFailure.disconnected;
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
