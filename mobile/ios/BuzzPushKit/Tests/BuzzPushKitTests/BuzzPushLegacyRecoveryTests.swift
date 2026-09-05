import Foundation
import Testing

@testable import BuzzPushKit

struct BuzzPushLegacyRecoveryTests {
  @Test func extractsGatewayNeutralRecoveryMaterial() throws {
    let grants = try #require(
      """
      [{"relayOrigin":"wss://relay.example/","endpointGrant":"opaque-grant","ignored":true}]
      """.data(using: .utf8)
    )
    let pending = try #require(
      """
      [{"relayOrigin":"https://pending.example","endpointHash":"abc","appProfile":"buzz-ios-dogfood","expiresAt":99}]
      """.data(using: .utf8)
    )

    let inventory = try BuzzPushLegacyRecoveryInventory.decode(
      grants: grants,
      pending: pending
    )

    #expect(
      inventory.relayOrigins == ["wss://pending.example", "wss://relay.example"]
    )
    #expect(inventory.endpointGrants == ["opaque-grant"])
    #expect(inventory.pendingEnrollments.map(\.endpointHash) == ["abc"])
  }

  @Test func rejectsInvalidRelayOriginsWithoutAssigningGatewayAuthority() throws {
    let grants = try #require(
      """
      [{"relayOrigin":"wss://relay.example/path","endpointGrant":"opaque-grant"}]
      """.data(using: .utf8)
    )

    #expect(throws: (any Error).self) {
      try BuzzPushLegacyRecoveryInventory.decode(grants: grants, pending: nil)
    }
  }
}
