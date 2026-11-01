import 'dart:io' show Platform;

import 'package:cupertino_http/cupertino_http.dart';
import 'package:http/http.dart' as http;
import 'package:objective_c/objective_c.dart';
import 'package:flutter/foundation.dart' show visibleForTesting;

/// Which app-wide connection pool a request belongs to.
enum HttpTraffic {
  /// Relay queries, NIP-11 documents, account and admission calls.
  api,

  /// Image, video and voice-note downloads and media uploads.
  media,
}

final _sessions = <HttpTraffic, http.Client>{};

/// An HTTP client for [traffic]. Callers own it and close it as usual.
///
/// On iOS, every client for the same [traffic] shares one `NSURLSession`, so
/// requests to a host multiplex over one HTTP/2 connection instead of opening
/// an HTTP/1.1 connection each, and gzip is inflated natively, off the Dart
/// isolate. Media has its own session, and so its own connection, so image
/// bytes cannot starve queries queued behind them on a shared connection.
///
/// Elsewhere this is `http.Client()`, which also honors `http.runWithClient`.
http.Client newAppHttpClient([HttpTraffic traffic = HttpTraffic.api]) {
  if (!Platform.isIOS) return http.Client();
  return SharedSessionClient(_sessions[traffic] ??= _openUrlSession());
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

/// A caller's handle on an app-wide session. [close] leaves the session open
/// for other callers; closing a `CupertinoClient` while any of its requests
/// run would also throw.
///
/// App Transport Security blocks cleartext `URLSession` loads from public
/// hosts, while saved community icons and remote media may still use
/// `http://`. Those keep the handle's own `dart:io` client, as before.
@visibleForTesting
class SharedSessionClient extends http.BaseClient {
  SharedSessionClient(this._session, {http.Client Function()? cleartext})
    : _newCleartext = cleartext ?? http.Client.new;

  final http.Client _session;
  final http.Client Function() _newCleartext;
  http.Client? _cleartext;

  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) =>
      request.url.scheme == 'https'
      ? _session.send(request)
      : (_cleartext ??= _newCleartext()).send(request);

  @override
  void close() => _cleartext?.close();
}

/// How an iOS transport request failed, for user-facing messages. `dart:io`
/// failures are recognized by their messages instead.
enum TransportFailure { unreachable, insecure, timedOut }

/// The [TransportFailure] for a `URLSession` error, or null for any other
/// error.
TransportFailure? urlSessionFailure(Object error) {
  if (error is! NSErrorClientException) return null;
  if (error.error.domain.toDartString() != 'NSURLErrorDomain') return null;
  return urlErrorFailure(error.error.code);
}

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
