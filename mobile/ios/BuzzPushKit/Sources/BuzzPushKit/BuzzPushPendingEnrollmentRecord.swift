/// Crash-recovery journal written before installation or delegation requests.
/// The Keychain-backed journal retains the APNs endpoint only when an enrollment
/// request may need exact replay after response loss.
public struct BuzzPushPendingEnrollmentRecord: Codable, Equatable, Sendable {
  /// Gateway authority for which this retry journal remains valid.
  public let gatewayOrigin: String
  public let relayOrigin: String
  public let relayPubkey: String
  /// Protected APNs endpoint needed to replay an enrollment without the current token.
  public let endpoint: String?
  public let endpointHash: String
  public let appProfile: String
  public let expiresAt: Int64
  public let installationId: String
  public let gatewayInstallationHandle: String?
  public let challengeId: String?
  public let challenge: String?
  public let keyId: String?
  public let attestation: String?
  public let delegationGeneration: Int64
  /// Durable marker that the journaled delegation revocation completed remotely.
  public let delegationRevoked: Bool?

  public init(
    gatewayOrigin: String,
    relayOrigin: String,
    relayPubkey: String,
    endpoint: String? = nil,
    endpointHash: String,
    appProfile: String,
    expiresAt: Int64,
    installationId: String,
    gatewayInstallationHandle: String? = nil,
    challengeId: String? = nil,
    challenge: String? = nil,
    keyId: String? = nil,
    attestation: String? = nil,
    delegationGeneration: Int64 = 0,
    delegationRevoked: Bool? = nil
  ) {
    self.gatewayOrigin = gatewayOrigin
    self.relayOrigin = relayOrigin
    self.relayPubkey = relayPubkey
    self.endpoint = endpoint
    self.endpointHash = endpointHash
    self.appProfile = appProfile
    self.expiresAt = expiresAt
    self.installationId = installationId
    self.gatewayInstallationHandle = gatewayInstallationHandle
    self.challengeId = challengeId
    self.challenge = challenge
    self.keyId = keyId
    self.attestation = attestation
    self.delegationGeneration = delegationGeneration
    self.delegationRevoked = delegationRevoked
  }

  func withGatewayInstallationHandle(_ handle: String) -> Self {
    Self(
      gatewayOrigin: gatewayOrigin,
      relayOrigin: relayOrigin,
      relayPubkey: relayPubkey,
      endpoint: endpoint,
      endpointHash: endpointHash,
      appProfile: appProfile,
      expiresAt: expiresAt,
      installationId: installationId,
      gatewayInstallationHandle: handle,
      challengeId: challengeId,
      challenge: challenge,
      keyId: keyId,
      attestation: attestation,
      delegationGeneration: delegationGeneration,
      delegationRevoked: delegationRevoked
    )
  }

  func withDelegationRevoked() -> Self {
    Self(
      gatewayOrigin: gatewayOrigin,
      relayOrigin: relayOrigin,
      relayPubkey: relayPubkey,
      endpoint: endpoint,
      endpointHash: endpointHash,
      appProfile: appProfile,
      expiresAt: expiresAt,
      installationId: installationId,
      gatewayInstallationHandle: gatewayInstallationHandle,
      challengeId: challengeId,
      challenge: challenge,
      keyId: keyId,
      attestation: attestation,
      delegationGeneration: delegationGeneration,
      delegationRevoked: true
    )
  }
}
