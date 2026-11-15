import 'dart:async';
import 'dart:io' show Platform;

import 'package:cronet_http/cronet_http.dart';
import 'package:cupertino_http/cupertino_http.dart';
import 'package:http/http.dart' as http;
import 'package:jni/jni.dart' show JThrowable;
import 'package:objective_c/objective_c.dart';
import 'package:flutter/foundation.dart' show debugPrint, visibleForTesting;

/// Which app-wide connection pool a request belongs to.
enum HttpTraffic {
  /// Relay queries, NIP-11 documents, account and admission calls.
  api,

  /// Image, video and voice-note downloads and media uploads.
  media,
}

/// The app-wide session for each traffic class, or null where the platform
/// has none.
final _sessions = <HttpTraffic, http.Client?>{};

/// An HTTP client for [traffic]. Callers own it and close it as usual.
///
/// On iOS and Android, every client for the same [traffic] shares one native
/// session (`NSURLSession` or a Cronet engine), so requests to a host
/// multiplex over one HTTP/2 connection instead of opening an HTTP/1.1
/// connection each, and gzip is inflated natively, off the Dart isolate.
/// Media has its own session, and so its own connection, so image bytes
/// cannot starve queries queued behind them on a shared connection.
///
/// Elsewhere this is `http.Client()`, which also honors `http.runWithClient`.
http.Client newAppHttpClient([HttpTraffic traffic = HttpTraffic.api]) {
  final session = _sessions.putIfAbsent(traffic, _openNativeSession);
  if (session == null) return http.Client();
  return SharedSessionClient(session, streamsUploads: !Platform.isAndroid);
}

/// Whether [newAppHttpClient] uses a native session on this device.
bool get hasNativeHttpSession =>
    _sessions.putIfAbsent(HttpTraffic.api, _openNativeSession) != null;

http.Client? _openNativeSession() {
  if (Platform.isIOS) return _openUrlSession();
  if (Platform.isAndroid) return _openCronetSession();
  return null;
}

/// Matches the `dart:io` transport apart from the protocol: no HTTP cache and
/// no cookies.
http.Client _openUrlSession() {
  final config = URLSessionConfiguration.ephemeralSessionConfiguration()
    ..cache = null
    ..httpShouldSetCookies = false
    ..httpCookieAcceptPolicy =
        NSHTTPCookieAcceptPolicy.NSHTTPCookieAcceptPolicyNever;
  return CupertinoClient.fromSessionConfiguration(config);
}

/// Matches the `dart:io` transport apart from the protocol: no HTTP cache,
/// and Cronet keeps no cookies. QUIC stays off, so this changes only the
/// HTTP version.
///
/// Null when no Cronet provider is available: Cronet comes from Google Play
/// services, which some Android devices don't have. Those keep `dart:io`.
http.Client? _openCronetSession() {
  final CronetEngine engine;
  try {
    engine = CronetEngine.build(
      cacheMode: CacheMode.disabled,
      enableHttp2: true,
      enableQuic: false,
    );
  } on JThrowable catch (error) {
    debugPrint('Cronet is unavailable, using dart:io: ${error.message}');
    return null;
  }
  return CronetClient.fromCronetEngine(engine);
}

/// A caller's handle on an app-wide session. Like closing a `dart:io` client,
/// [close] cancels the handle's requests still in flight and refuses later
/// ones, but leaves the session open for other handles. Closing a
/// `CupertinoClient` while any of its requests run would also throw, and
/// closing a `CronetClient` would stop the shared engine.
///
/// App Transport Security blocks cleartext `URLSession` loads from public
/// hosts, while saved community icons and remote media may still use
/// `http://`. Those keep the handle's own `dart:io` client, as before.
///
/// Unless [streamsUploads], a streamed request with a body (media and voice
/// note uploads) also keeps `dart:io`: `CronetClient` reads the whole body
/// into memory before sending it, so progress would reach 100% before any
/// byte left and a large attachment would be copied in full.
@visibleForTesting
class SharedSessionClient extends http.BaseClient {
  SharedSessionClient(
    this._session, {
    http.Client Function()? cleartext,
    this.streamsUploads = true,
  }) : _newCleartext = cleartext ?? http.Client.new;

  final http.Client _session;
  final http.Client Function() _newCleartext;

  /// Whether the session streams request bodies instead of buffering them.
  final bool streamsUploads;

  /// One abort trigger per request still running, so [close] can cancel it.
  /// A request leaves the set once its response body is done, failed or
  /// cancelled, or once the caller's own abort trigger fires. A body nobody
  /// listens to stays until then: the session keeps downloading it, so
  /// callers that drop a response unread must abort it.
  final _inFlight = <Completer<void>>{};
  var _closed = false;
  http.Client? _cleartext;

  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) async {
    if (_closed) {
      throw http.ClientException(
        'HTTP request failed. Client is already closed.',
        request.url,
      );
    }
    if (request.url.scheme != 'https' ||
        (!streamsUploads && _isStreamedUpload(request))) {
      return (_cleartext ??= _newCleartext()).send(request);
    }
    final abort = Completer<void>();
    void fire() {
      _inFlight.remove(abort);
      if (!abort.isCompleted) abort.complete();
    }

    _inFlight.add(abort);
    if (request case http.Abortable(:final abortTrigger?)) {
      abortTrigger.then<void>((_) => fire(), onError: (_) => fire());
    }
    final http.StreamedResponse response;
    try {
      response = await _session.send(_withAbortTrigger(request, abort.future));
    } catch (_) {
      _inFlight.remove(abort);
      rethrow;
    }
    // A controller, not an async* wrapper: async* never listens to the
    // session's stream when cancelled before its first event, so the
    // session would keep downloading a body nobody reads.
    late final StreamSubscription<List<int>> inner;
    final body = StreamController<List<int>>(sync: true);
    body
      ..onListen = () {
        inner = response.stream.listen(
          body.add,
          onError: body.addError,
          onDone: () {
            _inFlight.remove(abort);
            body.close();
          },
        );
      }
      ..onPause = (() => inner.pause())
      ..onResume = (() => inner.resume())
      ..onCancel = () {
        _inFlight.remove(abort);
        return inner.cancel();
      };

    return http.StreamedResponse(
      http.ByteStream(body.stream),
      response.statusCode,
      contentLength: response.contentLength,
      request: response.request,
      headers: response.headers,
      isRedirect: response.isRedirect,
      persistentConnection: response.persistentConnection,
      reasonPhrase: response.reasonPhrase,
    );
  }

  static bool _isStreamedUpload(http.BaseRequest request) =>
      request is! http.Request &&
      request.method != 'GET' &&
      request.method != 'HEAD';

  /// A copy of [request] that the session cancels when [trigger] completes.
  http.BaseRequest _withAbortTrigger(
    http.BaseRequest request,
    Future<void> trigger,
  ) {
    if (request is http.Request) {
      return http.AbortableRequest(
          request.method,
          request.url,
          abortTrigger: trigger,
        )
        ..bodyBytes = request.bodyBytes
        ..headers.addAll(request.headers)
        ..followRedirects = request.followRedirects
        ..maxRedirects = request.maxRedirects
        ..persistentConnection = request.persistentConnection;
    }
    // Finalizing first lets a multipart request set its content type.
    final body = request.finalize();
    final copy =
        http.AbortableStreamedRequest(
            request.method,
            request.url,
            abortTrigger: trigger,
          )
          ..headers.addAll(request.headers)
          ..contentLength = request.contentLength
          ..followRedirects = request.followRedirects
          ..maxRedirects = request.maxRedirects
          ..persistentConnection = request.persistentConnection;
    // addStream keeps backpressure: the body is read only as the session
    // reads the copy, so upload progress tracks bytes the session has taken
    // (including its read-ahead buffers), as with plain cupertino_http.
    unawaited(copy.sink.addStream(body).whenComplete(copy.sink.close));
    return copy;
  }

  @override
  void close() {
    _closed = true;
    for (final abort in _inFlight.toList()) {
      if (!abort.isCompleted) abort.complete();
    }
    _inFlight.clear();
    _cleartext?.close();
  }
}

/// How a native transport request failed, for user-facing messages.
/// `dart:io` failures are recognized by their messages instead.
enum TransportFailure { unreachable, insecure, timedOut }

/// The [TransportFailure] for a `URLSession` or Cronet error, or null for
/// any other error.
TransportFailure? transportFailure(Object error) => switch (error) {
  NSErrorClientException(:final error)
      when error.domain.toDartString() == 'NSURLErrorDomain' =>
    urlErrorFailure(error.code),
  NetworkException(:final errorCode, :final cronetInternalErrorCode) =>
    cronetErrorFailure(errorCode, cronetInternalErrorCode),
  _ => null,
};

/// Classifies an `NSURLErrorDomain` code.
@visibleForTesting
TransportFailure? urlErrorFailure(int code) => switch (code) {
  -1001 => TransportFailure.timedOut,
  // Cannot find host, cannot connect, connection lost, DNS lookup failed,
  // international roaming off, cellular data not allowed, offline.
  -1003 ||
  -1004 ||
  -1005 ||
  -1006 ||
  -1018 ||
  -1020 ||
  -1009 => TransportFailure.unreachable,
  // Secure connection failed and the server certificate errors.
  >= -1206 && <= -1200 => TransportFailure.insecure,
  _ => null,
};

/// Classifies a Cronet `NetworkException` by its `ERROR_*` code and, for
/// `ERROR_OTHER`, Chromium's internal net error code.
@visibleForTesting
TransportFailure? cronetErrorFailure(int errorCode, int internalCode) =>
    switch (errorCode) {
      4 || 6 => TransportFailure.timedOut,
      // Host not resolved, offline, network changed, connection closed,
      // refused or reset, address unreachable.
      1 || 2 || 3 || 5 || 7 || 8 || 9 => TransportFailure.unreachable,
      // SSL protocol error, cipher mismatch, and the certificate errors.
      11
          when internalCode == -107 ||
              internalCode == -113 ||
              (internalCode <= -200 && internalCode >= -299) =>
        TransportFailure.insecure,
      _ => null,
    };
