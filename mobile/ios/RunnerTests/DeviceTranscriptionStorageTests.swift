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
    try DeviceTranscriptionPlugin.excludeDraftsFromBackup(in: support)
    #expect(try drafts.resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup == true)
    #expect(try Data(contentsOf: audio) == bytes)
  }
}
