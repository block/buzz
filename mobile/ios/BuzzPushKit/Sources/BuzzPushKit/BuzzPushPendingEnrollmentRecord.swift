/// Crash-recovery journal written before installation or delegation requests.
/// The Keychain-backed journal retains the APNs endpoint only when an enrollment
/// request may need exact replay after response loss.
public struct BuzzPushPendingEnrollmentRecord: Codable, Equatable, Sendable {
  /// Gateway authority for which this retry journal remains valid.
  public let gatewayOrigin: String
  /// Canonical WebSocket relay origin that owns the client lease.
  public let relayOrigin: String
  /// Relay delegation key captured when the request was prepared.
  public let relayPubkey: String
  /// Protected APNs endpoint needed to replay an enrollment without the current token.
  public let endpoint: String
  /// Lowercase SHA-256 digest of the binary APNs device token.
  public let endpointHash: String
  /// Server-owned application identity used for enrollment.
  public let appProfile: String
  /// Unix expiration time of the installation or delegation authority.
  public let expiresAt: Int64
  /// Unlinkable per-relay lease address, distinct from the gateway handle.
  public let installationId: String
  /// Gateway installation identifier, available after enrollment succeeds.
  public let gatewayInstallationHandle: String?
  /// Identifier of the journaled enrollment challenge.
  public let challengeId: String?
  /// Exact challenge bytes encoded as unpadded base64url.
  public let challenge: String?
  /// App Attest key that signed the journaled request.
  public let keyId: String?
  /// Exact attestation object retained for idempotent enrollment replay.
  public let attestation: String?
  /// Highest delegation generation reserved before a network request.
  public let delegationGeneration: Int64

  /// Creates a gateway-scoped durable record from authenticated enrollment state.
  public init(
    gatewayOrigin: String,
    relayOrigin: String,
    relayPubkey: String,
    endpoint: String,
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
  }

}
