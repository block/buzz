import 'dart:convert';

import 'package:buzz/shared/hosted/browser_auth.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:crypto/crypto.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUp(() => FlutterSecureStorage.setMockInitialValues({}));

  test('cold-start callback restores PKCE and intent, consumes once', () async {
    late Uri login;
    final first = BrowserAuth(
      openBrowser: (uri) async {
        login = uri;
        return null;
      },
    );
    await first.start({
      'relay': 'wss://example.communities.buzz.xyz',
      'code': 'invite',
    });
    await first.dispose();
    final restored = BrowserAuth();
    final codes = <BrowserAuthCode>[];
    final failures = <BrowserAuthFailure>[];
    restored.codes.listen(codes.add);
    restored.failures.listen(failures.add);
    final callback = Uri.parse(BrowserAuth.callback).replace(
      queryParameters: {
        'state': login.queryParameters['state']!,
        'code': 'exchange-code',
      },
    );
    await restored.receive(callback);
    await restored.receive(callback);
    await Future<void>.delayed(Duration.zero);
    expect(codes, hasLength(1));
    expect(failures, isEmpty);
    expect(codes.single.intent['code'], 'invite');
    expect(
      login.queryParameters['code_challenge'],
      base64UrlEncode(
        sha256.convert(ascii.encode(codes.single.verifier)).bytes,
      ).replaceAll('=', ''),
    );
    expect(login.queryParameters['code_challenge_method'], 'S256');
    expect(login.origin, HostedEnvironment.origin.origin);
    final stored = await const FlutterSecureStorage().readAll();
    expect(stored.keys, everyElement(contains(HostedEnvironment.origin.host)));
    expect(login.queryParameters['returnTo'], BrowserAuth.callback);
    expect(await restored.retainedIntent(), codes.single.intent);
    await restored.dispose();
  });

  test(
    'invalid state and destination never consume the valid pending login',
    () async {
      late Uri login;
      final auth = BrowserAuth(
        openBrowser: (uri) async {
          login = uri;
          return null;
        },
      );
      final codes = <BrowserAuthCode>[];
      final failures = <BrowserAuthFailure>[];
      auth.codes.listen(codes.add);
      auth.failures.listen(failures.add);
      await auth.start({'relay': 'wss://example.communities.buzz.xyz'});
      final state = login.queryParameters['state']!;
      for (final invalid in [
        '${BrowserAuth.callback}?code=x',
        '${BrowserAuth.callback}?state=wrong&code=x',
        '${BrowserAuth.callback}?state=$state&state=$state&code=x',
        'xyz.block.buzz.mobile://auth/callback?state=$state&code=x',
        '${BrowserAuth.callback}/?state=$state&code=x',
        '${BrowserAuth.callback}?state=$state&code=x#fragment',
      ]) {
        await auth.receive(Uri.parse(invalid));
      }
      await Future<void>.delayed(Duration.zero);
      expect(codes, isEmpty);
      expect(failures, everyElement(BrowserAuthFailure.restartLogin));
      expect(failures, isEmpty);
      expect(await auth.retainedIntent(), isNotNull);
      await auth.receive(
        Uri.parse('${BrowserAuth.callback}?state=$state&code=good'),
      );
      await Future<void>.delayed(Duration.zero);
      expect(codes.single.code, 'good');
      await auth.dispose();
    },
  );

  test('expiry and cancel preserve destination but prevent exchange', () async {
    var now = DateTime.utc(2026);
    late Uri login;
    final auth = BrowserAuth(
      now: () => now,
      openBrowser: (uri) async {
        login = uri;
        return null;
      },
    );
    final codes = <BrowserAuthCode>[];
    auth.codes.listen(codes.add);
    await auth.start({'relay': 'wss://example.communities.buzz.xyz'});
    now = now.add(const Duration(minutes: 11));
    await auth.receive(
      Uri.parse(
        '${BrowserAuth.callback}?state=${login.queryParameters['state']}&code=x',
      ),
    );
    await auth.cancel();
    now = now.subtract(const Duration(minutes: 11));
    await auth.receive(
      Uri.parse(
        '${BrowserAuth.callback}?state=${login.queryParameters['state']}&code=x',
      ),
    );
    await Future<void>.delayed(Duration.zero);
    expect(codes, isEmpty);
    expect(await auth.retainedIntent(), isNotNull);
    await auth.dispose();
  });
  test('old callback cannot fail a newer pending login', () async {
    final logins = <Uri>[];
    final auth = BrowserAuth(
      openBrowser: (uri) async {
        logins.add(uri);
        return null;
      },
    );
    final failures = <BrowserAuthFailure>[];
    final codes = <BrowserAuthCode>[];
    auth.failures.listen(failures.add);
    auth.codes.listen(codes.add);
    await auth.start({'target': 'old'});
    await auth.start({'target': 'new'});
    await auth.receive(
      Uri.parse(
        '${BrowserAuth.callback}?state=${logins.first.queryParameters['state']}&code=old',
      ),
    );
    await Future<void>.delayed(Duration.zero);
    expect(failures, isEmpty);
    expect(codes, isEmpty);
    await auth.receive(
      Uri.parse(
        '${BrowserAuth.callback}?state=${logins.last.queryParameters['state']}&code=new',
      ),
    );
    await Future<void>.delayed(Duration.zero);
    expect(codes.single.intent['target'], 'new');
    await auth.dispose();
  });
}
