part of 'pairing_provider.dart';

enum PairingStatus {
  idle,
  connecting,
  confirmingSas,
  transferring,
  storing,
  success,
  error,
}

class PairingState {
  final PairingStatus status;
  final String? errorMessage;
  final String? sasCode;
  final bool userConfirmedSas;
  final bool sendsIdentityToDesktop;
  final bool protectSensitiveActions;
  final bool authorizationInProgress;
  final String? destinationRelayUrl;
  final bool requiresDesktopCode;

  const PairingState({
    this.status = PairingStatus.idle,
    this.errorMessage,
    this.sasCode,
    this.userConfirmedSas = false,
    this.sendsIdentityToDesktop = false,
    this.protectSensitiveActions = true,
    this.authorizationInProgress = false,
    this.destinationRelayUrl,
    this.requiresDesktopCode = false,
  });

  PairingState copyWith({
    PairingStatus? status,
    String? errorMessage,
    String? sasCode,
    bool? userConfirmedSas,
    bool? sendsIdentityToDesktop,
    bool? protectSensitiveActions,
    bool? authorizationInProgress,
    String? destinationRelayUrl,
    bool? requiresDesktopCode,
    bool clearErrorMessage = false,
  }) => PairingState(
    status: status ?? this.status,
    requiresDesktopCode: requiresDesktopCode ?? this.requiresDesktopCode,
    destinationRelayUrl: destinationRelayUrl ?? this.destinationRelayUrl,
    errorMessage: clearErrorMessage ? null : errorMessage ?? this.errorMessage,
    sasCode: sasCode ?? this.sasCode,
    userConfirmedSas: userConfirmedSas ?? this.userConfirmedSas,
    sendsIdentityToDesktop:
        sendsIdentityToDesktop ?? this.sendsIdentityToDesktop,
    protectSensitiveActions:
        protectSensitiveActions ?? this.protectSensitiveActions,
    authorizationInProgress:
        authorizationInProgress ?? this.authorizationInProgress,
  );
}
