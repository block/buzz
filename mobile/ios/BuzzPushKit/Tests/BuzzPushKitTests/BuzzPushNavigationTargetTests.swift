import Foundation
import Testing

@testable import BuzzPushKit

@Test func `Round-trip navigation target through notification user info`() {
  let target = BuzzPushNavigationTarget(
    eventID: "ABC123",
    communityID: "community-id",
    channelID: "channel-id"
  )

  #expect(
    BuzzPushNavigationTarget.decodeIfPresent(
      from: [BuzzPushNavigationTarget.userInfoKey: target.userInfoValue]
    ) == target
  )
  #expect(target.eventID == "abc123")
}

@Test func `Reject incomplete navigation target`() {
  #expect(
    BuzzPushNavigationTarget.decodeIfPresent(
      from: [
        BuzzPushNavigationTarget.userInfoKey: [
          "event_id": "event-id",
          "community_id": "community-id",
        ]
      ]
    ) == nil
  )
}

@Test func `Buffer preserves cold-start target until consumed`() {
  let first = BuzzPushNavigationTarget(
    eventID: "first",
    communityID: "community-id",
    channelID: "channel-id"
  )
  let second = BuzzPushNavigationTarget(
    eventID: "second",
    communityID: "community-id",
    channelID: "channel-id"
  )
  let buffer = BuzzPushNavigationBuffer()

  buffer.record(first)
  buffer.remove(ifMatching: second)
  #expect(buffer.peek() == first)
  #expect(buffer.take() == first)
  #expect(buffer.take() == nil)
}
