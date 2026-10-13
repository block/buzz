import Foundation
import Testing

@testable import BuzzPushKit

@Test func `Round-trip opaque navigation target through notification user info`() {
  let target = BuzzPushNavigationTarget(
    eventID: "MESSAGE-ID",
    communityID: "community-id",
    channelID: "CHANNEL/GENERAL"
  )

  #expect(
    BuzzPushNavigationTarget.decodeIfPresent(
      from: [BuzzPushNavigationTarget.userInfoKey: target.userInfoValue]
    ) == target
  )
  #expect(target.eventID == "MESSAGE-ID")
  #expect(target.channelID == "CHANNEL/GENERAL")
}

@Test func `Reject incomplete or malformed navigation target`() {
  #expect(
    BuzzPushNavigationTarget.decodeIfPresent(
      from: [
        BuzzPushNavigationTarget.userInfoKey: [
          "event_id": "message-id",
          "community_id": "community-id",
        ]
      ]
    ) == nil
  )

  #expect(
    BuzzPushNavigationTarget.decodeIfPresent(
      from: [
        BuzzPushNavigationTarget.userInfoKey: [
          "event_id": "",
          "community_id": "community-id",
          "channel_id": "channel-id",
        ]
      ]
    ) == nil
  )

  #expect(
    BuzzPushNavigationTarget.decodeIfPresent(
      from: [
        BuzzPushNavigationTarget.userInfoKey: [
          "event_id": "message-id",
          "community_id": "community-id",
          "channel_id": "",
        ]
      ]
    ) == nil
  )
}

@Test func `Buffer preserves cold-start target until consumed`() {
  let first = BuzzPushNavigationTarget(
    eventID: String(repeating: "a", count: 64),
    communityID: "community-id",
    channelID: "123e4567-e89b-42d3-a456-426614174000"
  )
  let second = BuzzPushNavigationTarget(
    eventID: String(repeating: "b", count: 64),
    communityID: "community-id",
    channelID: "123e4567-e89b-42d3-a456-426614174000"
  )
  let buffer = BuzzPushNavigationBuffer()

  buffer.record(first)
  buffer.remove(ifMatching: second)
  #expect(buffer.peek() == first)
  #expect(buffer.take() == first)
  #expect(buffer.take() == nil)
}

@Test func `Reply target preserves containing thread through user info and cold-start buffer`() {
  let target = BuzzPushNavigationTarget(
    eventID: "reply", communityID: "community", channelID: "channel", threadRootID: "parent"
  )
  let decoded = BuzzPushNavigationTarget.decodeIfPresent(
    from: [BuzzPushNavigationTarget.userInfoKey: target.userInfoValue]
  )
  #expect(decoded == target)
  let buffer = BuzzPushNavigationBuffer()
  buffer.record(target)
  #expect(buffer.take() == target)
}

@Test func `Reject malformed optional thread and unknown navigation fields`() {
  let cases: [[String: Any]] = [
    ["thread_root_id": ""], ["thread_root_id": 42], ["unknown": "value"],
  ]
  for extra in cases {
    var raw: [String: Any] = [
      "event_id": "reply", "community_id": "community", "channel_id": "channel",
    ]
    raw.merge(extra) { _, new in new }
    #expect(BuzzPushNavigationTarget.decodeIfPresent(
      from: [BuzzPushNavigationTarget.userInfoKey: raw]
    ) == nil)
  }
}
