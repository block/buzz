import 'dart:convert';
import 'dart:io';

/// What `mobile/scripts/nip_fi_live/fixture.py` publishes about the relay,
/// issuer and TLS front it started. Every value is ephemeral and generated
/// per run.
class NipFiFixture {
  NipFiFixture._(this._json, this._markers);

  /// Reads the fixture named by `BUZZ_NIP_FI_FIXTURE`. A missing fixture fails
  /// the test: a live check that silently has nothing to check proves nothing.
  factory NipFiFixture.load() {
    final path = Platform.environment['BUZZ_NIP_FI_FIXTURE'];
    if (path == null || path.isEmpty) {
      throw StateError(
        'BUZZ_NIP_FI_FIXTURE is not set: run mobile/scripts/nip_fi_live/run.sh',
      );
    }
    final json =
        jsonDecode(File(path).readAsStringSync()) as Map<String, dynamic>;
    return NipFiFixture._(json, File(json['markers'] as String));
  }

  final Map<String, dynamic> _json;
  final File _markers;

  String get relayOrigin => _json['relay_origin'] as String;
  String get secondOrigin => _json['second_origin'] as String;
  String get accountOrigin => _json['account_origin'] as String;
  String get nsec => _json['nsec'] as String;
  String get pubkey => _json['pubkey'] as String;
  int get assertionLifetimeSecs => _json['assertion_lifetime_secs'] as int;

  /// Records that [step] began, for the oracle to attribute relay log lines.
  void mark(String step) {
    _markers.writeAsStringSync(
      '${jsonEncode({'step': step, 't': DateTime.now().toUtc().microsecondsSinceEpoch / 1e6})}\n',
      mode: FileMode.append,
      flush: true,
    );
  }

  /// Sends every connection to the fixture's TLS front and trusts only its
  /// throwaway CA, so the app's own `https://relay.nipfi.localhost` URLs, which
  /// its hosted gate requires, reach the local relay with no :443 bind, no
  /// `*.localhost` resolver and no change to production code. A connection to
  /// any other host fails: the check never touches the real network.
  HttpOverrides overrides() => _FrontOverrides(
    port: _json['front_port'] as int,
    hosts: {
      Uri.parse(relayOrigin).host,
      Uri.parse(secondOrigin).host,
      Uri.parse(accountOrigin).host,
    },
    ca: File(_json['ca_pem'] as String).readAsBytesSync(),
  );
}

class _FrontOverrides extends HttpOverrides {
  _FrontOverrides({required this.port, required this.hosts, required this.ca});

  final int port;
  final Set<String> hosts;
  final List<int> ca;

  @override
  HttpClient createHttpClient(SecurityContext? context) {
    final trust = SecurityContext(withTrustedRoots: false)
      ..setTrustedCertificatesBytes(ca);
    return super.createHttpClient(trust)
      ..connectionFactory = (uri, proxyHost, proxyPort) {
        if (!hosts.contains(uri.host)) {
          throw StateError('NIP-FI live check refuses to reach ${uri.host}');
        }
        // A factory's socket is used as returned, so TLS is done here, with
        // the name the app asked for: the certificate is checked against it
        // while the bytes go to the fixture's loopback port.
        return Future.value(
          ConnectionTask.fromSocket(
            Socket.connect(InternetAddress.loopbackIPv4, port).then(
              (socket) =>
                  SecureSocket.secure(socket, host: uri.host, context: trust),
            ),
            () {},
          ),
        );
      };
  }
}
