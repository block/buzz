import 'dart:io';

import 'package:device_info_plus/device_info_plus.dart';
import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:package_info_plus/package_info_plus.dart';

const _buzzClientHeaderName = 'Buzz-Client';
const _userAgentHeaderName = 'User-Agent';

/// Immutable HTTP identification headers for the Buzz mobile client.
@immutable
class ClientHeaders {
  final String appVersion;
  final String buzzClient;
  final String userAgent;

  const ClientHeaders({
    required this.appVersion,
    required this.buzzClient,
    required this.userAgent,
  });

  Map<String, String> get values {
    if (buzzClient.isEmpty && userAgent.isEmpty) return const {};
    if (buzzClient.isEmpty || userAgent.isEmpty) {
      throw StateError('Buzz client headers must be initialized together');
    }
    return Map.unmodifiable({
      _buzzClientHeaderName: buzzClient,
      _userAgentHeaderName: userAgent,
    });
  }
}

/// Metadata used to construct [ClientHeaders].
@immutable
class ClientHeaderMetadata {
  final String platform;
  final String appVersion;
  final String appBuild;
  final String osVersion;
  final int? osApi;

  const ClientHeaderMetadata({
    required this.platform,
    required this.appVersion,
    required this.appBuild,
    required this.osVersion,
    this.osApi,
  });
}

/// Builds the canonical structured `Buzz-Client` and display-only `User-Agent`.
ClientHeaders buildClientHeaders(ClientHeaderMetadata metadata) {
  if (metadata.platform != 'ios' && metadata.platform != 'android') {
    throw ArgumentError.value(
      metadata.platform,
      'platform',
      'must be ios or android',
    );
  }
  if (metadata.platform == 'android' &&
      (metadata.osApi == null || metadata.osApi! <= 0)) {
    throw ArgumentError('Android client headers require a positive osApi');
  }
  if (metadata.platform == 'ios' && metadata.osApi != null) {
    throw ArgumentError('iOS client headers must not include osApi');
  }
  if (metadata.appBuild.isEmpty ||
      !metadata.appBuild.codeUnits.every(
        (codeUnit) => codeUnit >= 0x30 && codeUnit <= 0x39,
      )) {
    throw FormatException('App build must contain only decimal digits');
  }

  final buzzClient = StringBuffer(
    'v=1, app=buzz-mobile, platform=${metadata.platform}, '
    'app-version=${_structuredString(metadata.appVersion)}, '
    'app-build=${_structuredString(metadata.appBuild)}, '
    'os-version=${_structuredString(metadata.osVersion)}',
  );
  if (metadata.osApi case final osApi?) {
    buzzClient.write(', os-api=$osApi');
  }

  return ClientHeaders(
    appVersion: metadata.appVersion,
    buzzClient: buzzClient.toString(),
    userAgent:
        'Buzz/${_userAgentProduct(metadata.appVersion)} '
        '(${metadata.platform}; build ${_userAgentComment(metadata.appBuild)})',
  );
}

String _structuredString(String value) {
  final escaped = StringBuffer('"');
  for (final codeUnit in value.codeUnits) {
    if (codeUnit < 0x20 || codeUnit > 0x7e) {
      throw FormatException(
        'Buzz-Client structured strings must contain visible ASCII',
      );
    }
    if (codeUnit == 0x22 || codeUnit == 0x5c) {
      escaped.writeCharCode(0x5c);
    }
    escaped.writeCharCode(codeUnit);
  }
  escaped.write('"');
  return escaped.toString();
}

String _userAgentProduct(String value) {
  if (value.isEmpty || !_isHttpToken(value)) {
    throw FormatException(
      'App version must be a valid User-Agent product token',
    );
  }
  return value;
}

String _userAgentComment(String value) {
  if (value.isEmpty ||
      value.codeUnits.any(
        (codeUnit) => codeUnit < 0x20 || codeUnit > 0x7e || codeUnit == 0x29,
      )) {
    throw FormatException('App build must be valid in a User-Agent comment');
  }
  return value;
}

bool _isHttpToken(String value) {
  const separators = '()<>@,;:\\"/[]?={} \t';
  return value.codeUnits.every(
    (codeUnit) =>
        codeUnit >= 0x21 &&
        codeUnit <= 0x7e &&
        !separators.contains(String.fromCharCode(codeUnit)),
  );
}

/// Loads immutable mobile identification headers before the app starts.
Future<ClientHeaders> loadClientHeaders() async {
  final packageInfo = await PackageInfo.fromPlatform();
  final deviceInfo = DeviceInfoPlugin();

  if (Platform.isIOS) {
    final ios = await deviceInfo.iosInfo;
    return buildClientHeaders(
      ClientHeaderMetadata(
        platform: 'ios',
        appVersion: packageInfo.version,
        appBuild: packageInfo.buildNumber,
        osVersion: _coarseOsVersion(ios.systemVersion),
      ),
    );
  }
  if (Platform.isAndroid) {
    final android = await deviceInfo.androidInfo;
    return buildClientHeaders(
      ClientHeaderMetadata(
        platform: 'android',
        appVersion: packageInfo.version,
        appBuild: packageInfo.buildNumber,
        osVersion: _coarseOsVersion(android.version.release),
        osApi: android.version.sdkInt,
      ),
    );
  }

  throw UnsupportedError('Buzz mobile supports only iOS and Android');
}

/// Process-cached identification headers, overridden with platform data in main.
///
/// Tests and component previews do not execute [main], so the provider has an
/// inert value until the app-level override installs real platform metadata.
const _uninitializedClientHeaders = ClientHeaders(
  appVersion: '',
  buzzClient: '',
  userAgent: '',
);

final clientHeadersProvider = Provider<ClientHeaders>(
  (ref) => _uninitializedClientHeaders,
);

String _coarseOsVersion(String version) {
  final match = RegExp(r'^(\d+)(?:\.(\d+))?').firstMatch(version.trim());
  if (match == null) {
    throw FormatException('OS version does not start with a numeric release');
  }
  final minor = match.group(2);
  return minor == null ? match.group(1)! : '${match.group(1)}.$minor';
}

/// Returns true when [targetUrl] is a Buzz-owned first-party origin.
///
/// The active relay origin is trusted explicitly. Hosted Buzz community and
/// pairing domains are trusted by suffix/exact match. Local origins are only
/// trusted in debug builds.
bool isFirstPartyBuzzUrl(
  String targetUrl, {
  String? relayUrl,
  bool allowLocalDevelopment = kDebugMode,
}) {
  final target = Uri.tryParse(targetUrl);
  if (target == null || target.host.isEmpty || !_isHttpOrWebSocket(target)) {
    return false;
  }

  if (relayUrl case final configuredRelay?) {
    final relay = Uri.tryParse(configuredRelay);
    if (relay != null && _sameOrigin(target, relay)) return true;
  }

  final host = target.host.toLowerCase();
  if (host == 'pairing.buzz.xyz' || host.endsWith('.communities.buzz.xyz')) {
    return target.scheme == 'https' || target.scheme == 'wss';
  }

  if (allowLocalDevelopment && _isLocalHost(host)) {
    return true;
  }
  return false;
}

Map<String, String> clientHeadersForUrl({
  required ClientHeaders headers,
  required String targetUrl,
  String? relayUrl,
  bool allowLocalDevelopment = kDebugMode,
}) {
  if (!isFirstPartyBuzzUrl(
    targetUrl,
    relayUrl: relayUrl,
    allowLocalDevelopment: allowLocalDevelopment,
  )) {
    return const {};
  }
  return headers.values;
}

bool _isHttpOrWebSocket(Uri uri) =>
    uri.scheme == 'http' ||
    uri.scheme == 'https' ||
    uri.scheme == 'ws' ||
    uri.scheme == 'wss';

bool _sameOrigin(Uri a, Uri b) {
  final aSecure = _isSecureScheme(a.scheme);
  final bSecure = _isSecureScheme(b.scheme);
  return aSecure != null &&
      aSecure == bSecure &&
      a.host.toLowerCase() == b.host.toLowerCase() &&
      _effectivePort(a) == _effectivePort(b);
}

bool? _isSecureScheme(String scheme) => switch (scheme) {
  'https' || 'wss' => true,
  'http' || 'ws' => false,
  _ => null,
};

int? _effectivePort(Uri uri) {
  if (uri.hasPort) return uri.port;
  return switch (uri.scheme) {
    'https' || 'wss' => 443,
    'http' || 'ws' => 80,
    _ => null,
  };
}

bool _isLocalHost(String host) =>
    host == 'localhost' || host == '127.0.0.1' || host == '::1';
