part of 'pairing_provider.dart';

String _friendlyErrorMessage(Object error) {
  final message = error.toString();
  if (message.contains('SocketException') ||
      message.contains('Connection refused') ||
      message.contains('Network is unreachable') ||
      message.contains('No route to host') ||
      message.contains('Failed to connect')) {
    return 'Could not reach the pairing relay. Check your internet '
        'connection and VPN, then try again.';
  }
  if (error is PairingAuthException) {
    return 'The pairing relay rejected authentication. Try creating a new '
        'pairing code.';
  }
  if (error is StateError ||
      message.contains('Null check operator used on a null value')) {
    return 'Pairing stopped because of an internal error. Please try again.';
  }
  if (message.contains('HandshakeException') ||
      message.contains('CERTIFICATE_VERIFY_FAILED')) {
    return 'Secure connection failed. Check your network settings '
        'and try again.';
  }
  if (message.contains('TimeoutException') || message.contains('timed out')) {
    return 'Connection timed out. Check your internet connection and '
        'try again.';
  }
  return 'Connection failed. Please check your internet connection '
      'and try again.';
}

void _validateRelayUrl(String url) {
  final uri = Uri.parse(url);

  if (!kDebugMode && uri.scheme != 'https') {
    throw const FormatException('Relay URL must use HTTPS');
  }
  if (uri.scheme != 'http' && uri.scheme != 'https') {
    throw FormatException('Invalid URL scheme: ${uri.scheme}');
  }

  final host = uri.host.toLowerCase();
  if (host == 'localhost' || host == '127.0.0.1' || host == '::1') {
    if (!kDebugMode) {
      throw const FormatException('Relay URL cannot target localhost');
    }
    return;
  }

  final ip = Uri.tryParse('http://$host')?.host ?? host;
  if (_isPrivateHost(ip)) {
    throw const FormatException(
      'Relay URL cannot target private network addresses',
    );
  }
}

bool _isPrivateHost(String host) {
  final parts = host.split('.');
  if (parts.length != 4) return false;
  final octets = parts.map(int.tryParse).toList();
  if (octets.any((o) => o == null)) return false;

  final a = octets[0]!;
  final b = octets[1]!;

  if (a == 10) return true;
  if (a == 172 && b >= 16 && b <= 31) return true;
  if (a == 192 && b == 168) return true;
  if (a == 169 && b == 254) return true;
  return false;
}
