import Foundation
import Testing
@testable import Buzz

struct DeviceTranscriptionStorageTests {
  @Test @MainActor
  func `Existing recordings survive backup exclusion`() throws {
    let support = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: support) }
    let drafts = support.appendingPathComponent("bestie-transcripts", isDirectory: true)
    try FileManager.default.createDirectory(at: drafts, withIntermediateDirectories: true)
    let audio = drafts.appendingPathComponent("saved.wav")
    let bytes = Data([1, 2, 3])
    try bytes.write(to: audio)
    let blocker = support.appendingPathComponent("composer-transcripts")
    try Data().write(to: blocker)
    try DeviceTranscriptionPlugin.excludeDraftsFromBackup(in: support, directory: "bestie-transcripts")
    #expect(try drafts.resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup == true)
    #expect(try Data(contentsOf: audio) == bytes)
    #expect(throws: (any Error).self) {
      try DeviceTranscriptionPlugin.excludeDraftsFromBackup(in: support, directory: "composer-transcripts")
    }
    try FileManager.default.removeItem(at: blocker)
    try DeviceTranscriptionPlugin.excludeDraftsFromBackup(in: support, directory: "composer-transcripts")
    let composer = support.appendingPathComponent("composer-transcripts", isDirectory: true)
    #expect(try composer.resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup == true)
  }
}
