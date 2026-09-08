import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:local_auth/local_auth.dart';
import 'package:mocktail/mocktail.dart';
import 'package:buzz/shared/security/sensitive_action_authorizer.dart';

class _MockLocalAuthentication extends Mock implements LocalAuthentication {}

void main() {
  late _MockLocalAuthentication authentication;

  setUp(() {
    authentication = _MockLocalAuthentication();
    when(authentication.isDeviceSupported).thenAnswer((_) async => true);
    when(
      authentication.getAvailableBiometrics,
    ).thenAnswer((_) async => <BiometricType>[BiometricType.face]);
    when(
      () => authentication.authenticate(
        localizedReason: any(named: 'localizedReason'),
        authMessages: any(named: 'authMessages'),
        biometricOnly: any(named: 'biometricOnly'),
        sensitiveTransaction: any(named: 'sensitiveTransaction'),
        persistAcrossBackgrounding: any(named: 'persistAcrossBackgrounding'),
      ),
    ).thenAnswer((_) async => true);
  });

  test('identity authorization retries after system backgrounding', () async {
    final result = await LocalSensitiveActionAuthorizer(
      authentication,
    ).authorizeIdentityAction(biometricOnly: false);

    expect(result, DeviceAuthResult.success);
    verify(
      () => authentication.authenticate(
        localizedReason: 'Confirm sending your Buzz identity to desktop',
        authMessages: any(named: 'authMessages'),
        biometricOnly: false,
        sensitiveTransaction: true,
        persistAcrossBackgrounding: true,
      ),
    ).called(1);
  });

  test('unsupported devices fail closed before authentication', () async {
    when(authentication.isDeviceSupported).thenAnswer((_) async => false);

    final result = await LocalSensitiveActionAuthorizer(
      authentication,
    ).authorizeIdentityAction(biometricOnly: false);

    expect(result, DeviceAuthResult.unavailable);
    verifyNever(
      () => authentication.authenticate(
        localizedReason: any(named: 'localizedReason'),
        authMessages: any(named: 'authMessages'),
        biometricOnly: any(named: 'biometricOnly'),
        sensitiveTransaction: any(named: 'sensitiveTransaction'),
        persistAcrossBackgrounding: any(named: 'persistAcrossBackgrounding'),
      ),
    );
  });

  const mappedExceptions = <LocalAuthExceptionCode, DeviceAuthResult>{
    LocalAuthExceptionCode.userCanceled: DeviceAuthResult.cancelled,
    LocalAuthExceptionCode.systemCanceled: DeviceAuthResult.cancelled,
    LocalAuthExceptionCode.timeout: DeviceAuthResult.cancelled,
    LocalAuthExceptionCode.temporaryLockout: DeviceAuthResult.lockedOut,
    LocalAuthExceptionCode.biometricLockout: DeviceAuthResult.lockedOut,
    LocalAuthExceptionCode.noCredentialsSet: DeviceAuthResult.unavailable,
    LocalAuthExceptionCode.noBiometricsEnrolled: DeviceAuthResult.unavailable,
    LocalAuthExceptionCode.noBiometricHardware: DeviceAuthResult.unavailable,
    LocalAuthExceptionCode.biometricHardwareTemporarilyUnavailable:
        DeviceAuthResult.unavailable,
    LocalAuthExceptionCode.uiUnavailable: DeviceAuthResult.unavailable,
    LocalAuthExceptionCode.authInProgress: DeviceAuthResult.failed,
  };
  for (final MapEntry(key: code, value: expected) in mappedExceptions.entries) {
    test('maps ${code.name} to ${expected.name}', () async {
      when(
        () => authentication.authenticate(
          localizedReason: any(named: 'localizedReason'),
          authMessages: any(named: 'authMessages'),
          biometricOnly: any(named: 'biometricOnly'),
          sensitiveTransaction: any(named: 'sensitiveTransaction'),
          persistAcrossBackgrounding: any(named: 'persistAcrossBackgrounding'),
        ),
      ).thenThrow(LocalAuthException(code: code));

      final result = await LocalSensitiveActionAuthorizer(
        authentication,
      ).authorizeIdentityAction(biometricOnly: false);

      expect(result, expected);
    });
  }

  test('biometric protection requires an enrolled biometric', () async {
    when(
      authentication.getAvailableBiometrics,
    ).thenAnswer((_) async => <BiometricType>[]);

    final result = await LocalSensitiveActionAuthorizer(
      authentication,
    ).authorizeBiometricProtection();

    expect(result, DeviceAuthResult.unavailable);
    verifyNever(
      () => authentication.authenticate(
        localizedReason: any(named: 'localizedReason'),
        authMessages: any(named: 'authMessages'),
        biometricOnly: any(named: 'biometricOnly'),
        sensitiveTransaction: any(named: 'sensitiveTransaction'),
        persistAcrossBackgrounding: any(named: 'persistAcrossBackgrounding'),
      ),
    );
  });

  test(
    'biometric protection does not fall back to a device passcode',
    () async {
      final result = await LocalSensitiveActionAuthorizer(
        authentication,
      ).authorizeBiometricProtection();

      expect(result, DeviceAuthResult.success);
      verify(
        () => authentication.authenticate(
          localizedReason: 'Enable biometrics for secure actions',
          authMessages: any(named: 'authMessages'),
          biometricOnly: true,
          sensitiveTransaction: true,
          persistAcrossBackgrounding: true,
        ),
      ).called(1);
    },
  );

  test('single-flight shares authorization with the same mode', () async {
    final authorizer = _ControllableAuthorizer();
    final session = SensitiveActionAuthorizationSession(authorizer);

    final first = session.authorize(biometricOnly: false);
    final second = session.authorize(biometricOnly: false);

    expect(authorizer.biometricOnlyCalls, [false]);
    authorizer.completeNext(DeviceAuthResult.success);
    expect(await first, DeviceAuthResult.success);
    expect(await second, DeviceAuthResult.success);
  });

  test('single-flight does not reuse a weaker in-flight mode', () async {
    final authorizer = _ControllableAuthorizer();
    final session = SensitiveActionAuthorizationSession(authorizer);

    final passcodeAllowed = session.authorize(biometricOnly: false);
    final biometricOnly = session.authorize(biometricOnly: true);

    expect(authorizer.biometricOnlyCalls, [false]);
    authorizer.completeNext(DeviceAuthResult.success);
    expect(await passcodeAllowed, DeviceAuthResult.success);
    await Future<void>.delayed(Duration.zero);
    expect(authorizer.biometricOnlyCalls, [false, true]);

    authorizer.completeNext(DeviceAuthResult.success);
    expect(await biometricOnly, DeviceAuthResult.success);
  });
}

class _ControllableAuthorizer implements SensitiveActionAuthorizer {
  final biometricOnlyCalls = <bool>[];
  final _pending = <Completer<DeviceAuthResult>>[];

  void completeNext(DeviceAuthResult result) {
    _pending.removeAt(0).complete(result);
  }

  @override
  Future<DeviceAuthResult> authorizeIdentityAction({
    required bool biometricOnly,
  }) {
    biometricOnlyCalls.add(biometricOnly);
    final completer = Completer<DeviceAuthResult>();
    _pending.add(completer);
    return completer.future;
  }

  @override
  Future<DeviceAuthResult> authorizeBiometricProtection() =>
      throw UnimplementedError();

  @override
  Future<List<BiometricType>> enrolledBiometrics() =>
      throw UnimplementedError();
}
