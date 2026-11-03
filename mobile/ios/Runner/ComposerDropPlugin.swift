import Flutter
import UIKit
import UniformTypeIdentifiers

/// Accepts files dragged onto the app (from Files, Photos, another app in
/// Split View, or the Mac desktop when running on Apple silicon) and hands
/// sandbox copies to the active Flutter composer. Drops are only accepted
/// while Dart reports a mounted composer.
final class ComposerDropPlugin: NSObject, UIDropInteractionDelegate {
  static let directoryName = "composer-drops"

  private let channel: FlutterMethodChannel
  private var enabled = false
  private weak var attachedView: UIView?

  init(messenger: FlutterBinaryMessenger) {
    channel = FlutterMethodChannel(name: "buzz/composer_drop", binaryMessenger: messenger)
    super.init()
    Self.removeStaleDrops()
    channel.setMethodCallHandler { [weak self] call, result in
      guard call.method == "setEnabled" else {
        result(FlutterMethodNotImplemented)
        return
      }
      self?.enabled = (call.arguments as? Bool) ?? false
      result(nil)
    }
  }

  func attach(to view: UIView) {
    guard attachedView !== view else { return }
    view.addInteraction(UIDropInteraction(delegate: self))
    attachedView = view
  }

  // MARK: UIDropInteractionDelegate

  func dropInteraction(_ interaction: UIDropInteraction, canHandle session: UIDropSession) -> Bool {
    enabled && session.items.contains { Self.fileType(for: $0.itemProvider) != nil }
  }

  func dropInteraction(
    _ interaction: UIDropInteraction,
    sessionDidUpdate session: UIDropSession
  ) -> UIDropProposal {
    UIDropProposal(operation: enabled ? .copy : .forbidden)
  }

  func dropInteraction(_ interaction: UIDropInteraction, performDrop session: UIDropSession) {
    let providers = session.items.map(\.itemProvider)
    let group = DispatchGroup()
    let lock = NSLock()
    var files: [(index: Int, file: [String: String])] = []
    var failures = 0

    for (index, provider) in providers.enumerated() {
      guard let type = Self.fileType(for: provider) else { continue }
      group.enter()
      provider.loadFileRepresentation(forTypeIdentifier: type.identifier) { url, _ in
        defer { group.leave() }
        let copied = url.flatMap { Self.copyIntoSandbox($0, provider: provider, type: type) }
        lock.lock()
        if let copied {
          files.append((index, copied))
        } else {
          failures += 1
        }
        lock.unlock()
      }
    }

    group.notify(queue: .main) { [weak self] in
      let ordered = files.sorted { $0.index < $1.index }.map(\.file)
      self?.channel.invokeMethod("didDrop", arguments: ["files": ordered, "failures": failures])
    }
  }

  // MARK: Helpers

  /// The most specific file-backed type the provider offers. Plain text or
  /// link drags without a file name are skipped so a dragged sentence does not
  /// turn into a .txt attachment.
  static func fileType(for provider: NSItemProvider) -> UTType? {
    let types = provider.registeredTypeIdentifiers.compactMap(UTType.init)
    let isTextOnly = types.allSatisfy { $0.conforms(to: .text) || $0.conforms(to: .url) }
    if isTextOnly && provider.suggestedName == nil { return nil }
    return types.first { $0.conforms(to: .data) && !$0.conforms(to: .url) }
  }

  private static func copyIntoSandbox(
    _ source: URL,
    provider: NSItemProvider,
    type: UTType
  ) -> [String: String]? {
    let fileManager = FileManager.default
    let directory = fileManager.temporaryDirectory
      .appendingPathComponent(directoryName, isDirectory: true)
      .appendingPathComponent(UUID().uuidString, isDirectory: true)
    var name = provider.suggestedName ?? source.deletingPathExtension().lastPathComponent
    name = name.replacingOccurrences(of: "/", with: "_")
    if name.isEmpty { name = "Dropped file" }
    let ext = source.pathExtension.isEmpty ? (type.preferredFilenameExtension ?? "") : source.pathExtension
    if !ext.isEmpty && (name as NSString).pathExtension.lowercased() != ext.lowercased() {
      name += ".\(ext)"
    }
    let destination = directory.appendingPathComponent(name)
    do {
      try fileManager.createDirectory(at: directory, withIntermediateDirectories: true)
      try fileManager.copyItem(at: source, to: destination)
    } catch {
      return nil
    }
    let mimeType = UTType(filenameExtension: destination.pathExtension)?.preferredMIMEType
      ?? type.preferredMIMEType
      ?? "application/octet-stream"
    return ["path": destination.path, "name": name, "mimeType": mimeType]
  }

  /// Copies are owned by Dart and deleted after send or removal; this clears
  /// anything left behind by a process that was killed mid-draft.
  private static func removeStaleDrops() {
    let directory = FileManager.default.temporaryDirectory
      .appendingPathComponent(directoryName, isDirectory: true)
    try? FileManager.default.removeItem(at: directory)
  }
}
