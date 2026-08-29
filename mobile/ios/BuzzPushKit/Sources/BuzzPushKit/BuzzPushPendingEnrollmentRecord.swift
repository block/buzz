/// Crash-recovery journal written before installation or delegation requests.
/// It contains no APNs endpoint, only its hash and the exact authenticated
/// enrollment material needed to replay a committed request idempotently.
public struct BuzzPushPendingEnrollmentRecord: Codable, Equatable, Sendable {
  public let relayOrigin: String
  public let relayPubkey: String
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

  public init(
    relayOrigin: String,
    relayPubkey: String,
    endpointHash: String,
    appProfile: String,
    expiresAt: Int64,
    installationId: String,
    gatewayInstallationHandle: String? = nil,
    challengeId: String? = nil,
    challenge: String? = nil,
    keyId: String? = nil,
    attestation: String? = nil,
    delegationGeneration: Int64 = 0
  ) {
    self.relayOrigin = relayOrigin
    self.relayPubkey = relayPubkey
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
  }
}
