import Foundation
import Testing

@testable import BuzzPushKit

struct BuzzAgeRestrictionSessionTests {
  private func directory() throws -> URL {
    let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    return url
  }

  @Test func confirmedRestrictionIsReleasedWithoutAStorageWrite() throws {
    let url = try directory()
    defer { try? FileManager.default.removeItem(at: url) }
    let session = BuzzAgeRestrictionSession()
    #expect(!BuzzAgeRestrictionSession.isRestricted(containerURL: url))
    try session.restrict(containerURL: url)
    #expect(BuzzAgeRestrictionSession.isRestricted(containerURL: url))
    try session.restrict(containerURL: url)
    session.release()
    #expect(!BuzzAgeRestrictionSession.isRestricted(containerURL: url))
  }

  @Test func missingOrStaleReadOnlyFilesCannotRestrict() throws {
    let url = try directory()
    defer { try? FileManager.default.removeItem(at: url) }
    #expect(!BuzzAgeRestrictionSession.isRestricted(containerURL: nil))
    #expect(!BuzzAgeRestrictionSession.isRestricted(containerURL: url))
    let file = url.appendingPathComponent(BuzzAgeRestrictionSession.fileName)
    try Data("old or malformed restriction".utf8).write(to: file)
    try FileManager.default.setAttributes([.posixPermissions: 0o400], ofItemAtPath: file.path)
    #expect(!BuzzAgeRestrictionSession.isRestricted(containerURL: url))
  }

  @Test func acquisitionFailureCannotCreateAuthority() {
    let missing = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    let session = BuzzAgeRestrictionSession()
    #expect(throws: (any Error).self) { try session.restrict(containerURL: missing) }
    #expect(!BuzzAgeRestrictionSession.isRestricted(containerURL: missing))
  }

  @Test func `Pending restriction prevents newer handoffs before the current delivery finishes`() throws {
    let url = try directory()
    defer { try? FileManager.default.removeItem(at: url) }
    let session = BuzzAgeRestrictionSession()
    var delivered = false
    let allowed = BuzzAgeRestrictionSession.handoffIfAllowed(containerURL: url) {
      #expect(throws: (any Error).self) { try session.restrict(containerURL: url) }
      #expect(BuzzAgeRestrictionSession.isRestricted(containerURL: url))
      #expect(!BuzzAgeRestrictionSession.handoffIfAllowed(containerURL: url) {
        Issue.record("Pending restriction must prevent a newer handoff")
      })
      delivered = true
    }
    #expect(allowed)
    #expect(delivered)
    try session.restrict(containerURL: url)
    #expect(!BuzzAgeRestrictionSession.handoffIfAllowed(containerURL: url) {
      Issue.record("A confirmed restriction must suppress ordinary content")
    })
    session.release()
  }

  @Test func `Allowed restoration does not wait for a suspended handoff`() throws {
    let url = try directory()
    defer { try? FileManager.default.removeItem(at: url) }
    let session = BuzzAgeRestrictionSession()
    let allowed = BuzzAgeRestrictionSession.handoffIfAllowed(containerURL: url) {
      #expect(throws: (any Error).self) { try session.restrict(containerURL: url) }
      session.release()
      #expect(!BuzzAgeRestrictionSession.isRestricted(containerURL: url))
      #expect(BuzzAgeRestrictionSession.handoffIfAllowed(containerURL: url) {})
    }
    #expect(allowed)
  }

  @Test func `Handoff delivers when restriction storage is unavailable`() {
    var deliveries = 0
    #expect(BuzzAgeRestrictionSession.handoffIfAllowed(containerURL: nil) { deliveries += 1 })
    let missing = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    #expect(BuzzAgeRestrictionSession.handoffIfAllowed(containerURL: missing) { deliveries += 1 })
    #expect(deliveries == 2)
  }

  #if os(macOS)
    @Test(.timeLimit(.minutes(1))) func processExitReleasesAuthorityWithoutCleanup() throws {
      let url = try directory()
      defer { try? FileManager.default.removeItem(at: url) }
      let process = Process()
      let input = Pipe()
      let output = Pipe()
      process.executableURL = URL(fileURLWithPath: "/usr/bin/python3")
      process.arguments = ["-c", """
        import fcntl, os, sys
        fd = os.open(sys.argv[1], os.O_CREAT | os.O_RDWR, 0o600)
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        os.write(1, b'1')
        os.read(0, 1)
        os._exit(0)
        """, url.appendingPathComponent(BuzzAgeRestrictionSession.fileName).path]
      process.standardInput = input
      process.standardOutput = output
      try process.run()
      defer { if process.isRunning { process.terminate() } }
      #expect(try output.fileHandleForReading.read(upToCount: 1) == Data("1".utf8))
      #expect(BuzzAgeRestrictionSession.isRestricted(containerURL: url))
      try input.fileHandleForWriting.close()
      process.waitUntilExit()
      #expect(process.terminationStatus == 0)
      #expect(!BuzzAgeRestrictionSession.isRestricted(containerURL: url))
      #expect(FileManager.default.fileExists(atPath: url.appendingPathComponent(BuzzAgeRestrictionSession.fileName).path))
    }

    @Test(.timeLimit(.minutes(1))) func nativeRestrictionIsVisibleToAnotherProcess() throws {
      let url = try directory()
      defer { try? FileManager.default.removeItem(at: url) }
      let session = BuzzAgeRestrictionSession()
      try session.restrict(containerURL: url)
      defer { session.release() }
      let process = Process()
      process.executableURL = URL(fileURLWithPath: "/usr/bin/python3")
      process.arguments = ["-c", """
        import fcntl, os, sys
        fd = os.open(sys.argv[1], os.O_RDONLY)
        try:
            fcntl.flock(fd, fcntl.LOCK_SH | fcntl.LOCK_NB)
        except BlockingIOError:
            sys.exit(0)
        sys.exit(1)
        """, url.appendingPathComponent(BuzzAgeRestrictionSession.fileName).path]
      try process.run()
      defer { if process.isRunning { process.terminate() } }
      process.waitUntilExit()
      #expect(process.terminationStatus == 0)
    }
  #endif
}
