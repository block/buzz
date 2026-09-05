import Foundation
import Testing

@testable import BuzzPushKit

struct BuzzPushCleanupTokenFenceTests {
  @Test
  func checkpointsWhenDeviceTokenIsStillCurrent() {
    let token = Data([0x01, 0x02])
    var checkpointed = false

    let didCheckpoint = BuzzPushCleanupTokenFence.checkpointIfCurrent(
      capturedDeviceToken: token,
      liveDeviceToken: token
    ) {
      checkpointed = true
    }

    #expect(didCheckpoint)
    #expect(checkpointed)
  }

  @Test
  func retainsWorkWhenDeviceTokenRotates() {
    var checkpointed = false

    let didCheckpoint = BuzzPushCleanupTokenFence.checkpointIfCurrent(
      capturedDeviceToken: Data([0x01, 0x02]),
      liveDeviceToken: Data([0x03, 0x04])
    ) {
      checkpointed = true
    }

    #expect(!didCheckpoint)
    #expect(!checkpointed)
  }
}
