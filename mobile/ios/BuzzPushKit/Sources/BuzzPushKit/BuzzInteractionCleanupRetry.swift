import Darwin
import Foundation

/// A durable retry for deleting restricted communication interactions.
/// This journal never grants authority to restrict app or notification access.
public final class BuzzInteractionCleanupRetry {
  private let containerURL: URL?
  private let deletion: BuzzInteractionDeletionDeadline

  /// Uses the shared app-group directory and a bounded platform deletion.
  public init(containerURL: URL?, deletion: BuzzInteractionDeletionDeadline) {
    self.containerURL = containerURL
    self.deletion = deletion
  }

  /// Records intent before starting deletion. Failed or timed-out deletion
  /// leaves the record available to a subsequent app process.
  public func request(completion: @escaping (Error?) -> Void) {
    do {
      let token = UUID()
      guard let containerURL else { throw CocoaError(.fileNoSuchFile) }
      // Writers never wait for acknowledgements. Atomic replacement records
      // newer intent even when another process holds the completion lock.
      try Data(token.uuidString.utf8).write(to: recordURL(containerURL), options: .atomic)
      delete(token: token, completion: completion)
    } catch { completion(error) }
  }

  /// Retries only recorded cleanup; normal startup creates no cleanup intent.
  public func retryPending(completion: @escaping (Error?) -> Void) {
    guard containerURL != nil else { completion(nil); return }
    do {
      guard let containerURL,
        let token = try readToken(recordURL(containerURL)),
        token != (try completedToken(containerURL))
      else {
        completion(nil)
        return
      }
      delete(token: token, completion: completion)
    } catch { completion(error) }
  }

  private func delete(token: UUID, completion: @escaping (Error?) -> Void) {
    deletion.deleteAll { [self] error in
      if let error { completion(error); return }
      do {
        try withLock { directory in
          // Never remove intent: a concurrent writer may replace it at any
          // instant. Acknowledging this token leaves any newer token pending.
          // Serialize acknowledgements so an older completion cannot overwrite
          // a newer acknowledgement after checking the requested token.
          if try readToken(recordURL(directory)) == token {
            try Data(token.uuidString.utf8).write(to: completedURL(directory), options: .atomic)
          }
        }
        completion(nil)
      } catch { completion(error) }
    }
  }

  private func recordURL(_ directory: URL) -> URL {
    directory.appendingPathComponent("interaction-cleanup-retry")
  }

  private func completedURL(_ directory: URL) -> URL {
    directory.appendingPathComponent("interaction-cleanup-completed")
  }

  private func completedToken(_ directory: URL) throws -> UUID? {
    do {
      return try readToken(completedURL(directory))
    } catch let error as CocoaError where error.code == .fileReadCorruptFile {
      // A damaged acknowledgement is not evidence that deletion succeeded.
      return nil
    }
  }

  private func readToken(_ url: URL) throws -> UUID? {
    let handle: FileHandle
    do {
      handle = try FileHandle(forReadingFrom: url)
    } catch let error as CocoaError where error.code == .fileReadNoSuchFile || error.code == .fileNoSuchFile {
      return nil
    }
    defer { try? handle.close() }
    let bytes = try handle.read(upToCount: 64) ?? Data()
    guard bytes.count == 36, let value = String(data: bytes, encoding: .utf8),
      let token = UUID(uuidString: value)
    else { throw CocoaError(.fileReadCorruptFile) }
    return token
  }

  private func withLock<T>(_ body: (URL) throws -> T) throws -> T {
    guard let containerURL else { throw CocoaError(.fileNoSuchFile) }
    let path = containerURL.appendingPathComponent("interaction-cleanup-retry.lock").path
    let descriptor = open(path, O_CREAT | O_RDWR, S_IRUSR | S_IWUSR)
    guard descriptor >= 0 else { throw posixError() }
    defer { close(descriptor) }
    guard flock(descriptor, LOCK_EX | LOCK_NB) == 0 else { throw posixError() }
    defer { flock(descriptor, LOCK_UN) }
    return try body(containerURL)
  }

  private func posixError() -> POSIXError {
    POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO)
  }
}
