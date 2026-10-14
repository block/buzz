#if os(iOS)
import BuzzPushKit
import UserNotifications
import XCTest

final class BuzzCommunicationDeletionTimeoutTests: XCTestCase {
  func testLateDonationDeletionTimeoutCompletesOnce() {
    let ordinary = UNMutableNotificationContent()
    var allowed = true
    var timeout: (() -> Void)?
    var deletionCallback: ((Error?) -> Void)?
    var completions = 0
    var failures = 0
    let presenter = BuzzCommunicationNotificationPresenter(
      donate: { _, completion in
        allowed = false
        completion(nil)
      },
      deleteAllInteractions: { deletionCallback = $0 },
      updateContent: { content, _ in
        XCTFail("Restricted donation must not update content")
        return content
      },
      scheduleDeletionTimeout: { delay, action in
        XCTAssertEqual(delay, 5)
        timeout = action
      }
    )
    presenter.present(
      ordinaryContent: ordinary,
      resolution: communicationResolution(),
      isStillAllowed: { allowed },
      onDeletionFailure: { _ in failures += 1 }
    ) { _ in completions += 1 }
    XCTAssertEqual(completions, 0)
    XCTAssertNotNil(timeout)
    timeout?()
    XCTAssertEqual(failures, 1)
    XCTAssertEqual(completions, 1)
    deletionCallback?(nil)
    XCTAssertEqual(failures, 1)
    XCTAssertEqual(completions, 1)
  }

  private func communicationResolution(
    displayName: String = "Alice",
    groupName: String? = "General",
    avatarPNG: Data? = nil,
    recipientCount: Int? = 1
  ) -> BuzzPushResolution {
    let communityID = "community-id"
    let channelID = "channel/general:v5"
    return BuzzPushResolution(
      title: displayName,
      body: "Hello Buzz",
      subtitle: "Community",
      threadIdentifier: BuzzPushPresentationIdentity.conversation(
        communityID: communityID,
        channelID: channelID
      ),
      navigationTarget: BuzzPushNavigationTarget(
        eventID: "message-id",
        communityID: communityID,
        channelID: channelID
      ),
      senderPubkey: String(repeating: "a", count: 64),
      senderAvatarPNG: avatarPNG,
      conversationIdentifier: BuzzPushPresentationIdentity.conversation(
        communityID: communityID,
        channelID: channelID
      ),
      conversationDisplayName: groupName,
      conversationRecipientCount: recipientCount
    )
  }
}
#endif
