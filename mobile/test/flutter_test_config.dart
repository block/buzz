import 'dart:async';

import 'package:buzz/shared/relay/relay_provider.dart';

/// Runs every test file as a push-capable build, so a plain `flutter test`
/// needs no `--dart-define=BUZZ_PUSH_GATEWAY_URL`. Tests of a build without a
/// gateway set [Env.debugPushGatewayUrlOverride] to `''` themselves.
Future<void> testExecutable(FutureOr<void> Function() testMain) async {
  Env.debugPushGatewayUrlOverride = 'https://push.example';
  await testMain();
}
