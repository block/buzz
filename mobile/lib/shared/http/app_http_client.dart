import 'dart:io' show Platform;

import 'package:cupertino_http/cupertino_http.dart';
import 'package:http/http.dart' as http;
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
@visibleForTesting
class SharedSessionClient extends http.BaseClient {
  SharedSessionClient(this._session);

  final http.Client _session;

  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) =>
      _session.send(request);

  @override
  void close() {}
}
