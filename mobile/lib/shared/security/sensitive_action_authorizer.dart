import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:local_auth/local_auth.dart';

/// Coarse outcomes safe to use for control flow without retaining OS details.
enum DeviceAuthResult { success, cancelled, unavailable, lockedOut, failed }

abstract interface class SensitiveActionAuthorizer {
  Future<DeviceAuthResult> authorizeIdentityAction({
    required bool biometricOnly,
  });

  Future<DeviceAuthResult> authorizeBiometricProtection();

  Future<List<BiometricType>> enrolledBiometrics();
}

class LocalSensitiveActionAuthorizer implements SensitiveActionAuthorizer {
  LocalSensitiveActionAuthorizer([LocalAuthentication? authentication])
    : _authentication = authentication ?? LocalAuthentication();

  final LocalAuthentication _authentication;

  @override
  Future<DeviceAuthResult> authorizeIdentityAction({
    required bool biometricOnly,
  }) => _authorize(
    localizedReason: 'Confirm sending your Buzz identity to desktop',
    biometricOnly: biometricOnly,
  );

  @override
  Future<List<BiometricType>> enrolledBiometrics() async {
    try {
      return await _authentication.getAvailableBiometrics();
    } catch (_) {
      return const [];
    }
  }

  @override
  Future<DeviceAuthResult> authorizeBiometricProtection() async {
    try {
      final availableBiometrics = await _authentication
          .getAvailableBiometrics();
      if (availableBiometrics.isEmpty) return DeviceAuthResult.unavailable;
      return _authorize(
        localizedReason: 'Enable biometrics for secure actions',
        biometricOnly: true,
      );
    } on LocalAuthException catch (error) {
      return _resultFor(error);
    } catch (_) {
      return DeviceAuthResult.failed;
    }
  }

  Future<DeviceAuthResult> _authorize({
    required String localizedReason,
    required bool biometricOnly,
  }) async {
    try {
      final supported = await _authentication.isDeviceSupported();
      if (!supported) return DeviceAuthResult.unavailable;
      final authenticated = await _authentication.authenticate(
        localizedReason: localizedReason,
        biometricOnly: biometricOnly,
        sensitiveTransaction: true,
        // iOS may briefly background the app while presenting Face ID. Keep
        // this authorization alive across that system transition instead of
        // returning a cancellation that forces the user to tap and retry.
        persistAcrossBackgrounding: true,
      );
      return authenticated ? DeviceAuthResult.success : DeviceAuthResult.failed;
    } on LocalAuthException catch (error) {
      return _resultFor(error);
    } catch (_) {
      return DeviceAuthResult.failed;
    }
  }

  static DeviceAuthResult _resultFor(LocalAuthException error) =>
      switch (error.code) {
        LocalAuthExceptionCode.userCanceled ||
        LocalAuthExceptionCode.systemCanceled ||
        LocalAuthExceptionCode.timeout => DeviceAuthResult.cancelled,
        LocalAuthExceptionCode.temporaryLockout ||
        LocalAuthExceptionCode.biometricLockout => DeviceAuthResult.lockedOut,
        LocalAuthExceptionCode.noCredentialsSet ||
        LocalAuthExceptionCode.noBiometricsEnrolled ||
        LocalAuthExceptionCode.noBiometricHardware ||
        LocalAuthExceptionCode.biometricHardwareTemporarilyUnavailable ||
        LocalAuthExceptionCode.uiUnavailable => DeviceAuthResult.unavailable,
        _ => DeviceAuthResult.failed,
      };
}

final sensitiveActionAuthorizerProvider = Provider<SensitiveActionAuthorizer>((
  ref,
) {
  return LocalSensitiveActionAuthorizer();
});

final enrolledBiometricsProvider = FutureProvider<List<BiometricType>>((ref) {
  return ref.watch(sensitiveActionAuthorizerProvider).enrolledBiometrics();
});

String biometricProtectionLabel(
  TargetPlatform platform,
  Iterable<BiometricType> enrolledBiometrics,
) {
  if (platform == TargetPlatform.iOS) {
    if (enrolledBiometrics.contains(BiometricType.face)) return 'Use Face ID';
    if (enrolledBiometrics.contains(BiometricType.fingerprint)) {
      return 'Use Touch ID';
    }
  }
  return 'Use biometrics';
}

class SensitiveActionAuthorizationSession {
  SensitiveActionAuthorizationSession(this._authorizer);

  final SensitiveActionAuthorizer _authorizer;
  Future<DeviceAuthResult>? _authorizationInFlight;
  bool? _inFlightBiometricOnly;

  Future<DeviceAuthResult> authorize({required bool biometricOnly}) async {
    final inFlight = _authorizationInFlight;
    if (inFlight != null) {
      if (_inFlightBiometricOnly == biometricOnly) return inFlight;
      await inFlight;
      return authorize(biometricOnly: biometricOnly);
    }

    final authorization = _authorizer.authorizeIdentityAction(
      biometricOnly: biometricOnly,
    );
    _authorizationInFlight = authorization;
    _inFlightBiometricOnly = biometricOnly;
    try {
      final result = await authorization;
      return result;
    } finally {
      if (identical(_authorizationInFlight, authorization)) {
        _authorizationInFlight = null;
        _inFlightBiometricOnly = null;
      }
    }
  }
}

final sensitiveActionAuthorizationSessionProvider =
    Provider<SensitiveActionAuthorizationSession>((ref) {
      return SensitiveActionAuthorizationSession(
        ref.watch(sensitiveActionAuthorizerProvider),
      );
    });
