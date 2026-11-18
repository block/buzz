import Darwin
import Foundation
import Testing

@testable import BuzzPushKit

struct BuzzInteractionCleanupRetryTests {
  private func directory() throws -> URL {
    let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    return url
  }

  private func retry(_ url: URL?, delete: @escaping BuzzInteractionDeletionDeadline.DeleteAllInteractions)
    -> BuzzInteractionCleanupRetry {
    BuzzInteractionCleanupRetry(containerURL: url, deletion: BuzzInteractionDeletionDeadline(
      timeout: 5, deleteAllInteractions: delete, scheduleTimeout: { _, _ in }))
  }

  @Test func `Failed cleanup survives recreation without restricting access`() throws {
    let url = try directory()
    defer { try? FileManager.default.removeItem(at: url) }
    let failure = NSError(domain: "InjectedFailure", code: 1)
    var failures = 0
    retry(url, delete: { $0(failure) }).request { error in
      #expect(error != nil)
      failures += 1
    }
    #expect(failures == 1)
    #expect(!BuzzAgeRestrictionSession.isRestricted(containerURL: url))
    var deletes = 0
    retry(url, delete: { completion in deletes += 1; completion(nil) }).retryPending {
      #expect($0 == nil)
    }
    #expect(deletes == 1)
    retry(url, delete: { _ in Issue.record("Successful cleanup must clear its retry") })
      .retryPending { #expect($0 == nil) }
  }

  @Test func `An older success cannot erase newer pending cleanup`() throws {
    let url = try directory()
    defer { try? FileManager.default.removeItem(at: url) }
    var first: ((Error?) -> Void)?
    var second: ((Error?) -> Void)?
    retry(url, delete: { first = $0 }).request { #expect($0 == nil) }
    retry(url, delete: { second = $0 }).request { #expect($0 != nil) }
    first?(nil)
    second?(NSError(domain: "LaterDeletionFailed", code: 1))
    var deletes = 0
    retry(url, delete: { completion in deletes += 1; completion(nil) })
      .retryPending { #expect($0 == nil) }
    #expect(deletes == 1)
  }

  @Test func `An older completion cannot undo a newer acknowledgement`() throws {
    let url = try directory()
    defer { try? FileManager.default.removeItem(at: url) }
    var first: ((Error?) -> Void)?
    retry(url, delete: { first = $0 }).request { #expect($0 == nil) }
    retry(url, delete: { $0(nil) }).request { #expect($0 == nil) }
    first?(nil)
    retry(url, delete: { _ in Issue.record("Newer cleanup was already acknowledged") })
      .retryPending { #expect($0 == nil) }
  }

  @Test func `A damaged acknowledgement leaves cleanup retryable`() throws {
    let url = try directory()
    defer { try? FileManager.default.removeItem(at: url) }
    retry(url, delete: { $0(nil) }).request { #expect($0 == nil) }
    try Data("damaged".utf8).write(
      to: url.appendingPathComponent("interaction-cleanup-completed"), options: .atomic)
    var deletes = 0
    retry(url, delete: { completion in deletes += 1; completion(nil) })
      .retryPending { #expect($0 == nil) }
    #expect(deletes == 1)
  }

  @Test func `A busy acknowledgement cannot lose newer cleanup intent`() throws {
    let url = try directory()
    defer { try? FileManager.default.removeItem(at: url) }
    let lockPath = url.appendingPathComponent("interaction-cleanup-retry.lock").path
    let descriptor = open(lockPath, O_CREAT | O_RDWR, S_IRUSR | S_IWUSR)
    #expect(descriptor >= 0)
    defer { close(descriptor) }
    #expect(flock(descriptor, LOCK_EX | LOCK_NB) == 0)
    var deletes = 0
    retry(url, delete: { completion in deletes += 1; completion(nil) }).request {
      #expect($0 != nil)
    }
    #expect(deletes == 1)
    #expect(!BuzzAgeRestrictionSession.isRestricted(containerURL: url))
    flock(descriptor, LOCK_UN)
    retry(url, delete: { completion in deletes += 1; completion(nil) }).retryPending {
      #expect($0 == nil)
    }
    #expect(deletes == 2)
    retry(url, delete: { _ in Issue.record("Acknowledged retry must not repeat") })
      .retryPending { #expect($0 == nil) }
  }

  @Test func `Missing deletion callback remains pending after its deadline`() throws {
    let url = try directory()
    defer { try? FileManager.default.removeItem(at: url) }
    var timeout: (() -> Void)?
    var failures = 0
    let deletion = BuzzInteractionDeletionDeadline(timeout: 5,
      deleteAllInteractions: { _ in }, scheduleTimeout: { _, action in timeout = action })
    BuzzInteractionCleanupRetry(containerURL: url, deletion: deletion).request {
      #expect($0 != nil)
      failures += 1
    }
    timeout?()
    #expect(failures == 1)
    var deletes = 0
    retry(url, delete: { completion in deletes += 1; completion(nil) })
      .retryPending { #expect($0 == nil) }
    #expect(deletes == 1)
    #expect(!BuzzAgeRestrictionSession.isRestricted(containerURL: url))
  }

  @Test func `Normal startup creates no deletion and absent storage cannot restrict`() throws {
    let url = try directory()
    defer { try? FileManager.default.removeItem(at: url) }
    retry(url, delete: { _ in Issue.record("No cleanup was requested") })
      .retryPending { #expect($0 == nil) }
    retry(nil, delete: { _ in Issue.record("No shared store is configured") })
      .retryPending { #expect($0 == nil) }
    let missing = url.appendingPathComponent("missing-directory")
    retry(missing, delete: { _ in Issue.record("Journal failure must propagate") })
      .request { #expect($0 != nil) }
    #expect(!BuzzAgeRestrictionSession.isRestricted(containerURL: missing))
  }
}
