import BuzzPushKit
import Flutter
import Intents
import UserNotifications
import XCTest

@testable import Buzz

final class BuzzCommunicationNotificationTests: XCTestCase {
  func testIntentUsesVerifiedSenderAvatarAndGroupName() throws {
    let resolution = communicationResolution(
      displayName: "Alice",
      groupName: "General",
      avatarPNG: Data([0x89, 0x50, 0x4E, 0x47])
    )
    let descriptor = try XCTUnwrap(
      BuzzCommunicationNotificationDescriptor(resolution: resolution)
    )

    let intent = BuzzCommunicationNotificationPresenter.makeIntent(descriptor)

    XCTAssertEqual(intent.sender?.displayName, "Alice")
    XCTAssertEqual(intent.sender?.customIdentifier, descriptor.senderIdentifier)
    XCTAssertNotNil(intent.sender?.image)
    XCTAssertNotNil(intent.image(forParameterNamed: \.speakableGroupName))
    XCTAssertEqual(intent.content, "Hello Buzz")
    XCTAssertEqual(intent.speakableGroupName?.spokenPhrase, "General")
    XCTAssertEqual(intent.conversationIdentifier, resolution.conversationIdentifier)
    XCTAssertNil(intent.recipients)
    XCTAssertEqual(descriptor.recipientCount, 1)
    XCTAssertEqual(
      (intent.donationMetadata as? INSendMessageIntentDonationMetadata)?.recipientCount,
      1
    )
  }

  func testDirectMessageRetainsSenderAvatarWithoutGroupMetadata() throws {
    let resolution = communicationResolution(
      displayName: "Alice",
      groupName: nil,
      avatarPNG: Data([0x89, 0x50, 0x4E, 0x47])
    )
    let descriptor = try XCTUnwrap(
      BuzzCommunicationNotificationDescriptor(resolution: resolution)
    )

    let intent = BuzzCommunicationNotificationPresenter.makeIntent(descriptor)

    XCTAssertEqual(intent.sender?.displayName, "Alice")
    XCTAssertNotNil(intent.sender?.image)
    XCTAssertNil(intent.image(forParameterNamed: \.speakableGroupName))
    XCTAssertNil(intent.speakableGroupName)
    XCTAssertNil(intent.donationMetadata)
  }

  func testMissingVerifiedRecipientCountUsesOrdinaryPresentation() {
    XCTAssertNil(
      BuzzCommunicationNotificationDescriptor(
        resolution: communicationResolution(recipientCount: nil)
      )
    )
  }

  func testPresentationFallsBackWhenDonationFails() {
    let ordinary = UNMutableNotificationContent()
    ordinary.title = "Alice"
    ordinary.body = "Hello Buzz"
    var updateCalled = false
    let presenter = BuzzCommunicationNotificationPresenter(
      donate: { _, completion in
        completion(NSError(domain: "test", code: 1))
      },
      deleteAllInteractions: { completion in completion(nil) },
      updateContent: { content, _ in
        updateCalled = true
        return content
      }
    )
    let completed = expectation(description: "ordinary fallback returned")

    presenter.present(
      ordinaryContent: ordinary,
      resolution: communicationResolution()
    ) { content in
      XCTAssertEqual(content.title, "Alice")
      XCTAssertEqual(content.body, "Hello Buzz")
      completed.fulfill()
    }

    wait(for: [completed], timeout: 1)
    XCTAssertFalse(updateCalled)
  }

  func testPresentationDonatesBeforeSpecializing() {
    let ordinary = UNMutableNotificationContent()
    ordinary.title = "Alice"
    var order: [String] = []
    let presenter = BuzzCommunicationNotificationPresenter(
      donate: { _, completion in
        order.append("donate")
        completion(nil)
      },
      deleteAllInteractions: { completion in completion(nil) },
      updateContent: { _, _ in
        order.append("update")
        let specialized = UNMutableNotificationContent()
        specialized.title = "specialized"
        return specialized
      }
    )
    let completed = expectation(description: "specialized content returned")

    presenter.present(
      ordinaryContent: ordinary,
      resolution: communicationResolution()
    ) { content in
      XCTAssertEqual(content.title, "specialized")
      completed.fulfill()
    }

    wait(for: [completed], timeout: 1)
    XCTAssertEqual(order, ["donate", "update"])
  }

  func testPresentationFallsBackWhenSpecializationFails() {
    let ordinary = UNMutableNotificationContent()
    ordinary.title = "Alice"
    ordinary.body = "Hello Buzz"
    let presenter = BuzzCommunicationNotificationPresenter(
      donate: { _, completion in completion(nil) },
      deleteAllInteractions: { completion in completion(nil) },
      updateContent: { _, _ in throw NSError(domain: "test", code: 2) }
    )
    let completed = expectation(description: "ordinary fallback returned")

    presenter.present(
      ordinaryContent: ordinary,
      resolution: communicationResolution()
    ) { content in
      XCTAssertEqual(content.title, "Alice")
      XCTAssertEqual(content.body, "Hello Buzz")
      completed.fulfill()
    }

    wait(for: [completed], timeout: 1)
  }

  func testFenceChangeDeletesDonatedInteractionBeforeCompleting() {
    let ordinary = UNMutableNotificationContent()
    ordinary.title = "Alice"
    var allowed = true
    var order: [String] = []
    let presenter = BuzzCommunicationNotificationPresenter(
      donate: { _, completion in
        order.append("donate")
        allowed = false
        completion(nil)
      },
      deleteAllInteractions: { completion in
        order.append("delete")
        completion(nil)
      },
      updateContent: { content, _ in
        order.append("update")
        return content
      }
    )
    let completed = expectation(description: "restricted donation deleted")

    presenter.present(
      ordinaryContent: ordinary,
      resolution: communicationResolution(),
      isStillAllowed: { allowed }
    ) { content in
      order.append("complete")
      XCTAssertEqual(content.title, "Alice")
      completed.fulfill()
    }

    wait(for: [completed], timeout: 1)
    XCTAssertEqual(order, ["donate", "delete", "complete"])
  }

  func testBlockedFenceDoesNotDonate() {
    let ordinary = UNMutableNotificationContent()
    ordinary.title = "Alice"
    var donateCalled = false
    let presenter = BuzzCommunicationNotificationPresenter(
      donate: { _, completion in
        donateCalled = true
        completion(nil)
      },
      deleteAllInteractions: { completion in completion(nil) },
      updateContent: { content, _ in content }
    )
    let completed = expectation(description: "blocked donation skipped")

    presenter.present(
      ordinaryContent: ordinary,
      resolution: communicationResolution(),
      isStillAllowed: { false }
    ) { content in
      XCTAssertEqual(content.title, "Alice")
      completed.fulfill()
    }

    wait(for: [completed], timeout: 1)
    XCTAssertFalse(donateCalled)
  }

  func testFailedLateDonationDeletionReactivatesFenceUntilAppRetry() throws {
    let ordinary = UNMutableNotificationContent()
    ordinary.title = "Alice"
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(
      UUID().uuidString,
      isDirectory: true
    )
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let fenceStore = BuzzAgeRestrictionFenceStore(containerURL: directory)
    let fenceAtStart = fenceStore.current()
    var order: [String] = []
    let deletionFailure = NSError(domain: "test", code: 3)
    let presenter = BuzzCommunicationNotificationPresenter(
      donate: { _, completion in
        order.append("donate")
        XCTAssertNoThrow(try fenceStore.performFencedCleanup {})
        completion(nil)
      },
      deleteAllInteractions: { completion in
        order.append("delete")
        completion(deletionFailure)
      },
      updateContent: { content, _ in content }
    )
    let completed = expectation(description: "failed deletion fenced")

    presenter.present(
      ordinaryContent: ordinary,
      resolution: communicationResolution(),
      isStillAllowed: {
        !fenceStore.current().requiresDiscard(since: fenceAtStart)
      },
      onDeletionFailure: { error in
        XCTAssertEqual(error as NSError, deletionFailure)
        XCTAssertNoThrow(try fenceStore.begin())
        order.append("fence")
      }
    ) { content in
      order.append("complete")
      XCTAssertEqual(content.title, "Alice")
      completed.fulfill()
    }

    wait(for: [completed], timeout: 1)
    XCTAssertEqual(order, ["donate", "delete", "fence", "complete"])
    XCTAssertTrue(fenceStore.current().isFencing)

    var retryError: Error?
    try fenceStore.performFencedAsyncCleanup(
      { $0(nil) },
      completion: { retryError = $0 }
    )
    XCTAssertNil(retryError)
    XCTAssertFalse(fenceStore.current().isFencing)
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

final class BuzzPushSnapshotEnrichmentTests: XCTestCase {
  func testLaunchBeginsAgeRestrictionFenceBeforeFlutterState() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(
      UUID().uuidString,
      isDirectory: true
    )
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }

    try AppDelegate.beginLaunchAgeRestrictionFence(containerURL: directory)

    XCTAssertTrue(BuzzAgeRestrictionFenceStore(containerURL: directory).current().isFencing)
  }

  func testStrictAgeGateWriteFailsWhenAppGroupStoreIsUnavailable() {
    let bridge = BuzzPushSnapshotBridge(
      appGroupIdentifier: nil,
      endpointGrantStore: BuzzPushEndpointGrantKeychainStore(accessGroup: nil),
      keychainAccessGroup: nil
    )
    let completed = expectation(description: "strict write rejected")

    XCTAssertTrue(
      bridge.handle(
        FlutterMethodCall(
          methodName: "syncAgeGatePushSnapshot",
          arguments: [
            "section": "communities",
            "communities": [[String: Any]](),
            "signingKeys": [String: String](),
            "settleFence": false,
          ]
        )
      ) { value in
        XCTAssertEqual((value as? FlutterError)?.code, "snapshot_sync_unavailable")
        completed.fulfill()
      }
    )

    wait(for: [completed], timeout: 1)
  }

  func testMetadataAuthorityUsesCurrentAppProfileForMatchingRelay() {
    let correctProfile = grant(
      appProfile: BuzzDevPushEnrollmentDriver.appProfile,
      generation: 2,
      metadataPubkey: String(repeating: "a", count: 64)
    )
    let wrongProfile = grant(
      appProfile: "other-profile",
      generation: 99,
      metadataPubkey: String(repeating: "b", count: 64)
    )

    XCTAssertEqual(
      BuzzPushSnapshotBridge.relayMetadataPubkey(
        relayURL: "wss://relay.example/",
        grants: [wrongProfile, correctProfile]
      ),
      correctProfile.relayMetadataPubkey
    )
  }

  private func grant(
    appProfile: String,
    generation: Int64,
    metadataPubkey: String
  ) -> BuzzPushEndpointGrantRecord {
    BuzzPushEndpointGrantRecord(
      relayOrigin: "https://relay.example",
      relayPubkey: String(repeating: "c", count: 64),
      relayMetadataPubkey: metadataPubkey,
      installationId: "installation",
      endpointGrant: "opaque-grant",
      endpointHash: String(repeating: "d", count: 64),
      appProfile: appProfile,
      endpointEpoch: 1,
      generation: generation,
      expiresAt: 1_900_000_000
    )
  }
}

final class BuzzPushNotificationResponseTests: XCTestCase {
  func testValidDefaultActionRoutesAndCompletesExactlyOnce() {
    let target = BuzzPushNavigationTarget(
      eventID: "message-id",
      communityID: "community-id",
      channelID: "opaque-channel-id"
    )
    var routedTargets: [BuzzPushNavigationTarget] = []
    var forwarded = 0
    var completions = 0

    BuzzPushNotificationResponseCoordinator.handle(
      actionIdentifier: UNNotificationDefaultActionIdentifier,
      userInfo: [BuzzPushNavigationTarget.userInfoKey: target.userInfoValue],
      onTarget: { routedTargets.append($0) },
      forwardToFlutter: { pluginCompletion in
        forwarded += 1
        pluginCompletion()
        pluginCompletion()
      },
      completion: { completions += 1 }
    )

    XCTAssertEqual(routedTargets, [target])
    XCTAssertEqual(forwarded, 1)
    XCTAssertEqual(completions, 1)
  }

  func testMalformedTargetFallsBackToOneCompletion() {
    var routedTargets: [BuzzPushNavigationTarget] = []
    var completions = 0

    BuzzPushNotificationResponseCoordinator.handle(
      actionIdentifier: UNNotificationDefaultActionIdentifier,
      userInfo: [BuzzPushNavigationTarget.userInfoKey: ["event_id": ""]],
      onTarget: { routedTargets.append($0) },
      forwardToFlutter: { _ in },
      completion: { completions += 1 }
    )

    XCTAssertTrue(routedTargets.isEmpty)
    XCTAssertEqual(completions, 1)
  }

  func testNonDefaultActionIgnoresLateDuplicatePluginCompletion() throws {
    var routedTargets: [BuzzPushNavigationTarget] = []
    var pluginCompletion: (() -> Void)?
    var completions = 0

    BuzzPushNotificationResponseCoordinator.handle(
      actionIdentifier: "buzz.reply",
      userInfo: [:],
      onTarget: { routedTargets.append($0) },
      forwardToFlutter: { pluginCompletion = $0 },
      completion: { completions += 1 }
    )
    let capturedCompletion = try XCTUnwrap(pluginCompletion)
    capturedCompletion()
    capturedCompletion()

    XCTAssertTrue(routedTargets.isEmpty)
    XCTAssertEqual(completions, 1)
  }
}
