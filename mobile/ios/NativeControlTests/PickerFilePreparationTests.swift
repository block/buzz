import Foundation

@MainActor
private final class ExportGate {
  var pending: [Int: CheckedContinuation<String, Error>] = [:]
  var started: [Int] = []
  let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)

  init() throws { try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true) }

  func export(_ index: Int) async throws -> String {
    started.append(index)
    return try await withCheckedThrowingContinuation { pending[index] = $0 }
  }

  func waitFor(_ count: Int) async {
    while started.count < count { await Task.yield() }
  }

  func finish(_ index: Int) throws {
    let file = directory.appendingPathComponent("\(index).mov")
    try Data([UInt8(index)]).write(to: file)
    pending.removeValue(forKey: index)!.resume(returning: file.path)
  }
}

@main
struct PickerFilePreparationTests {
  @MainActor static func main() async throws {
    let gate = try ExportGate()
    defer { try? FileManager.default.removeItem(at: gate.directory) }
    let task = Task { try await PickerFilePreparation.prepare(count: 5, export: gate.export) }
    await gate.waitFor(3)
    precondition(gate.pending.count == 3, "Exports overlap but stay bounded")
    try gate.finish(2)
    await gate.waitFor(4)
    precondition(gate.pending.count == 3)
    try gate.finish(0)
    await gate.waitFor(5)
    for index in [4, 3, 1] { try gate.finish(index) }
    let paths = try await task.value
    precondition(paths.map { URL(fileURLWithPath: $0).lastPathComponent } == (0..<5).map { "\($0).mov" }, "Completion order must not reorder the selection")

    for cancel in [false, true] {
      let gate = try ExportGate()
      defer { try? FileManager.default.removeItem(at: gate.directory) }
      let task = Task { try await PickerFilePreparation.prepare(count: cancel ? 5 : 3, export: gate.export) }
      await gate.waitFor(3)
      if cancel { task.cancel() }
      else { gate.pending.removeValue(forKey: 0)!.resume(throwing: CocoaError(.fileReadUnknown)) }
      if cancel { try gate.finish(0) }
      // Providers may return files even after cancellation or another export fails.
      try gate.finish(1)
      try gate.finish(2)
      do {
        _ = try await task.value
        preconditionFailure("Failed/cancelled batches must not become attachments")
      } catch {}
      precondition(gate.started.count == 3, "Stop scheduling after failure or cancellation")
      let remaining = try FileManager.default.contentsOfDirectory(atPath: gate.directory.path)
      precondition(remaining.isEmpty, "Remove every completed temporary file")
    }
    print("Picker export batching: order, concurrency, failure and cancellation passed")
  }
}
