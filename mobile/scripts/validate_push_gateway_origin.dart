import 'dart:io';

bool isValidPushGatewayOrigin(String value, {bool requireHttps = false}) {
  try {
    final uri = Uri.parse(value);
    if (requireHttps
        ? uri.scheme != 'https'
        : uri.scheme != 'http' && uri.scheme != 'https') {
      return false;
    }
    if (!uri.hasAuthority || uri.host.isEmpty || uri.userInfo.isNotEmpty) {
      return false;
    }
    if (uri.path.isNotEmpty && uri.path != '/') return false;
    if (uri.hasQuery || uri.hasFragment) return false;
    if (requireHttps && uri.hasPort) return false;
    final port = uri.port;
    return port >= 1 && port <= 65535;
  } on FormatException {
    return false;
  }
}

void main(List<String> arguments) {
  final requireHttps =
      arguments.isNotEmpty && arguments.first == '--require-https';
  final values = requireHttps ? arguments.skip(1).toList() : arguments;
  if (values.length == 1 &&
      isValidPushGatewayOrigin(values.single, requireHttps: requireHttps)) {
    return;
  }
  final requirement = requireHttps
      ? 'an HTTPS origin without an explicit port, credentials, path, query, or fragment'
      : 'an HTTP(S) origin without credentials, path, query, or fragment';
  stderr.writeln('error: BUZZ_PUSH_GATEWAY_URL must be $requirement.');
  exitCode = 1;
}
