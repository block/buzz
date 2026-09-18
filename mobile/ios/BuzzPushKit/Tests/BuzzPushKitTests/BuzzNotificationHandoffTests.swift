import Foundation
import Testing

@testable import BuzzPushKit

struct BuzzNotificationHandoffTests {
  @Test func `Expiration delivers fallback before stalled cleanup and ignores late resolution`() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let session = BuzzAgeRestrictionSession()
    try session.restrict(containerURL: directory)
    defer { session.release() }
    var delivered: [String] = []
    var cleanupCount = 0
    var deletionCompletion: ((Error?) -> Void)?
    let deletion = BuzzInteractionDeletionDeadline(
      timeout: 5,
      deleteAllInteractions: { deletionCompletion = $0 },
      scheduleTimeout: { _, _ in }
    )
    let handoff = BuzzNotificationHandoff<String> { delivered.append($0) }
    func finish(_ content: String) {
      handoff.finish(
        content,
        restrictedFallback: "Open Buzz to view this message.",
        handoffIfAllowed: { deliver in
          BuzzAgeRestrictionSession.handoffIfAllowed(containerURL: directory, deliver: deliver)
        },
        cleanup: {
          // Neither the Intents callback nor its timer needs to fire for delivery.
          #expect(delivered == ["Open Buzz to view this message."])
          cleanupCount += 1
          deletion.deleteAll { _ in }
        }
      )
    }
    // Resolution is still pending when the system calls expiration.
    finish("Reconnect to your relay now")
    #expect(delivered == ["Open Buzz to view this message."])
    #expect(deletionCompletion != nil)
    // Expiration during pending cleanup, followed by a late resolver callback.
    finish("Reconnect to your relay now")
    finish("Private message")
    #expect(delivered == ["Open Buzz to view this message."])
    #expect(cleanupCount == 1)
  }

  @Test func `Unchanged fence delivers resolved content once without cleanup`() {
    var delivered: [String] = []
    let handoff = BuzzNotificationHandoff<String> { delivered.append($0) }
    for _ in 0..<2 {
      handoff.finish(
        "Resolved message",
        restrictedFallback: "Safe fallback",
        handoffIfAllowed: { deliver in deliver(); return true },
        cleanup: { Issue.record("Allowed delivery must not remove notifications") }
      )
    }
    #expect(delivered == ["Resolved message"])
  }
}
