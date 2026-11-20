import Flutter
import UIKit
import UniformTypeIdentifiers

/// Reads the user-selected document in place; never imports a plaintext copy
/// into Buzz's sandbox. The security-scoped grant lasts only for the read.
final class IdentityFilePlugin: NSObject, UIDocumentPickerDelegate,
  UIAdaptivePresentationControllerDelegate
{
  private let channel: FlutterMethodChannel
  private var pending: FlutterResult?
  private var requestID: UUID?
  private weak var activePicker: UIDocumentPickerViewController?

  init(messenger: FlutterBinaryMessenger) {
    channel = FlutterMethodChannel(name: "buzz/identity_file", binaryMessenger: messenger)
    super.init()
    Self.removeInterruptedExports()
    channel.setMethodCallHandler { [weak self] call, result in
      guard let self else {
        result(FlutterError(code: "import_failed", message: nil, details: nil))
        return
      }
      if call.method == "prepareExport" {
        self.prepareExport(call.arguments, result: result)
        return
      }
      guard call.method == "import" else {
        result(FlutterMethodNotImplemented)
        return
      }
      self.pick(result: result)
    }
  }

  /// Write the share source with complete protection from its first byte.
  /// Dart owns normal cleanup; startup handles a process killed during sharing.
  private func prepareExport(_ arguments: Any?, result: FlutterResult) {
    guard let arguments = arguments as? [String: Any],
      let text = arguments["text"] as? String,
      !text.isEmpty, text.utf8.count <= 1024,
      let root = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first
    else {
      result(FlutterError(code: "export_failed", message: nil, details: nil))
      return
    }
    let directory = root.appendingPathComponent("buzz-identity-\(UUID().uuidString)")
    do {
      try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
      let file = directory.appendingPathComponent("buzz-identity.nsec")
      try Data(text.utf8).write(to: file, options: [.atomic, .completeFileProtection])
      result(file.path)
    } catch {
      try? FileManager.default.removeItem(at: directory)
      result(FlutterError(code: "export_failed", message: nil, details: nil))
    }
  }

  /// A terminated share never reaches Dart's finally. Retire its sandbox copy
  /// on the next launch; active shares cannot exist while this engine starts.
  private static func removeInterruptedExports() {
    let manager = FileManager.default
    // path_provider_foundation resolves getTemporaryDirectory to Caches.
    let roots = manager.urls(for: .cachesDirectory, in: .userDomainMask)
      + [manager.temporaryDirectory]
    for root in roots {
      do {
        let entries = try manager.contentsOfDirectory(
          at: root,
          includingPropertiesForKeys: nil,
          options: [.skipsHiddenFiles]
        )
        for entry in entries where entry.lastPathComponent.hasPrefix("buzz-identity-") {
          try manager.removeItem(at: entry)
        }
      } catch {
        // No path or key material in diagnostics. Retry on the next launch.
        NSLog("Buzz could not clean up an interrupted identity export")
      }
    }
  }

  private func pick(result: @escaping FlutterResult) {
    guard pending == nil else {
      result(FlutterError(code: "import_busy", message: nil, details: nil))
      return
    }
    // The implicit Flutter engine can initialize before a view controller is
    // attached. Resolve the live scene at the user action, never cache the
    // registrar's possibly-nil launch-time view controller.
    let root = UIApplication.shared.connectedScenes
      .compactMap { $0 as? UIWindowScene }
      .filter { $0.activationState == .foregroundActive }
      .flatMap(\.windows)
      .first(where: \.isKeyWindow)?.rootViewController
    guard var top = root else {
      result(FlutterError(code: "import_failed", message: nil, details: nil))
      return
    }
    while true {
      if let presented = top.presentedViewController { top = presented }
      else if let navigation = top as? UINavigationController,
        let visible = navigation.visibleViewController { top = visible }
      else if let tabs = top as? UITabBarController,
        let selected = tabs.selectedViewController { top = selected }
      else { break }
    }
    guard !top.isBeingDismissed, top.viewIfLoaded?.window != nil else {
      result(FlutterError(code: "import_failed", message: nil, details: nil))
      return
    }
    let picker = UIDocumentPickerViewController(forOpeningContentTypes: [.item], asCopy: false)
    picker.allowsMultipleSelection = false
    picker.delegate = self
    pending = result
    requestID = UUID()
    activePicker = picker
    top.present(picker, animated: true)
    picker.presentationController?.delegate = self
  }

  func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {
    guard controller === activePicker else { return }
    guard let url = urls.first, urls.count == 1 else {
      finish(FlutterError(code: "import_failed", message: nil, details: nil))
      return
    }
    guard let requestID else { return }
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      let scoped = url.startAccessingSecurityScopedResource()
      defer { if scoped { url.stopAccessingSecurityScopedResource() } }
      var coordinationError: NSError?
      var text: String?
      NSFileCoordinator().coordinate(readingItemAt: url, options: [], error: &coordinationError) { accessible in
        text = Self.readBounded(accessible)
      }
      let value: Any? = coordinationError == nil && text != nil
        ? text : FlutterError(code: "import_failed", message: nil, details: nil)
      DispatchQueue.main.async {
        guard self?.requestID == requestID else { return }
        self?.finish(value)
      }
    }
  }

  /// Reads at most 1025 bytes, including the one-byte oversize sentinel.
  private static func readBounded(_ url: URL) -> String? {
    guard let stream = InputStream(url: url) else { return nil }
    stream.open()
    var bytes = [UInt8](repeating: 0, count: 1025)
    defer {
      stream.close()
      bytes.withUnsafeMutableBytes { $0.initializeMemory(as: UInt8.self, repeating: 0) }
    }
    var count = 0
    while count < bytes.count {
      let remaining = bytes.count - count
      let read = bytes.withUnsafeMutableBufferPointer { buffer in
        guard let base = buffer.baseAddress else { return -1 }
        return stream.read(base.advanced(by: count), maxLength: remaining)
      }
      if read < 0 { return nil }
      if read == 0 { break }
      count += read
    }
    guard count > 0, count <= 1024 else { return nil }
    return String(bytes: bytes.prefix(count), encoding: .utf8)
  }

  func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) {
    guard controller === activePicker else { return }
    finish(nil)
  }

  func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
    guard presentationController.presentedViewController === activePicker else { return }
    finish(nil)
  }

  private func finish(_ value: Any?) {
    let result = pending
    pending = nil
    requestID = nil
    activePicker = nil
    result?(value)
  }
}
