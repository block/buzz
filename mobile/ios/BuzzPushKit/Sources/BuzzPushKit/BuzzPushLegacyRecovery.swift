import Foundation

/// Gateway-neutral recovery material retained by the pre-gateway-origin schema.
public struct BuzzPushLegacyRecoveryInventory: Equatable {
  public let relayOrigins: [String]
  public let endpointGrants: [String]
  public let pendingEnrollments: [BuzzPushLegacyPendingRecovery]

  public init(
    relayOrigins: [String],
    endpointGrants: [String],
    pendingEnrollments: [BuzzPushLegacyPendingRecovery]
  ) {
    self.relayOrigins = relayOrigins
    self.endpointGrants = endpointGrants
    self.pendingEnrollments = pendingEnrollments
  }

  private struct LegacyGrant: Decodable {
    let relayOrigin: String
    let endpointGrant: String
  }

  public struct BuzzPushLegacyPendingRecovery: Decodable, Equatable {
    public let relayOrigin: String
    public let endpointHash: String
    public let appProfile: String
    public let expiresAt: Int64
    public let gatewayInstallationHandle: String?
    public let challengeId: String?
    public let challenge: String?
    public let keyId: String?
    public let attestation: String?

    public init(
      relayOrigin: String,
      endpointHash: String,
      appProfile: String,
      expiresAt: Int64,
      gatewayInstallationHandle: String? = nil,
      challengeId: String? = nil,
      challenge: String? = nil,
      keyId: String? = nil,
      attestation: String? = nil
    ) {
      self.relayOrigin = relayOrigin
      self.endpointHash = endpointHash
      self.appProfile = appProfile
      self.expiresAt = expiresAt
      self.gatewayInstallationHandle = gatewayInstallationHandle
      self.challengeId = challengeId
      self.challenge = challenge
      self.keyId = keyId
      self.attestation = attestation
    }
  }

  /// Extracts only gateway-neutral relay origins and opaque gateway proofs.
  /// No gateway origin or App Attest key is inferred for legacy records.
  public static func decode(grants: Data?, pending: Data?) throws -> Self {
    let legacyGrants =
      try grants.map {
        try JSONDecoder().decode([LegacyGrant].self, from: $0)
      } ?? []
    let legacyPending =
      try pending.map {
        try JSONDecoder().decode([BuzzPushLegacyPendingRecovery].self, from: $0)
      } ?? []
    let relayOrigins = try (legacyGrants.map(\.relayOrigin) + legacyPending.map(\.relayOrigin))
      .map { origin -> String in
        guard origin.utf8.count <= 2_048,
          var components = URLComponents(string: origin),
          components.host?.isEmpty == false,
          components.user == nil,
          components.password == nil,
          components.query == nil,
          components.fragment == nil,
          components.path.isEmpty || components.path == "/"
        else {
          throw CocoaError(.coderInvalidValue)
        }
        switch components.scheme?.lowercased() {
        case "https": components.scheme = "wss"
        case "http": components.scheme = "ws"
        case "wss", "ws": break
        default: throw CocoaError(.coderInvalidValue)
        }
        components.path = ""
        guard let canonical = components.string, canonical.utf8.count <= 2_048 else {
          throw CocoaError(.coderInvalidValue)
        }
        return canonical
      }
    let endpointGrants = legacyGrants.map(\.endpointGrant)
    guard endpointGrants.allSatisfy({ !$0.isEmpty && $0.utf8.count <= 4_096 }) else {
      throw CocoaError(.coderInvalidValue)
    }
    return Self(
      relayOrigins: Array(Set(relayOrigins)).sorted(),
      endpointGrants: Array(Set(endpointGrants)).sorted(),
      pendingEnrollments: legacyPending
    )
  }
}
