import ImageIO
import SwiftUI
import UIKit
import UniformTypeIdentifiers
import os.log

private let shareLog = Logger(subsystem: "xyz.block.buzz.share", category: "handoff")

/// Receives items from the iOS share sheet, lets the user pick a channel and
/// write a message right in the sheet, then stages everything in the App
/// Group inbox and hands off to the main app through `buzz://share?id=<id>`.
///
/// The extension never holds a relay session; the app does the actual
/// posting, opening straight into the chosen channel's composer.
final class ShareViewController: UIViewController {
  private let statusLabel = UILabel()
  private var didFinish = false
  private var didStartStaging = false
  private var stagedDirectory: URL?
  private var stagedID: String?
  private var stagedItems: [BuzzShareInbox.Item] = []

  private var appGroupIdentifier: String? {
    Bundle.main.object(forInfoDictionaryKey: "BuzzAppGroupIdentifier") as? String
  }

  override func viewDidLoad() {
    super.viewDidLoad()
    view.backgroundColor = .systemBackground
    statusLabel.text = "Preparing…"
    statusLabel.font = .preferredFont(forTextStyle: .body)
    statusLabel.textColor = .secondaryLabel
    statusLabel.translatesAutoresizingMaskIntoConstraints = false
    view.addSubview(statusLabel)
    NSLayoutConstraint.activate([
      statusLabel.centerXAnchor.constraint(equalTo: view.centerXAnchor),
      statusLabel.centerYAnchor.constraint(equalTo: view.centerYAnchor),
    ])
  }

  override func viewDidAppear(_ animated: Bool) {
    super.viewDidAppear(animated)
    // viewDidAppear can fire again (e.g. after a presented sheet); staging is
    // asynchronous, so guard on a flag set before it starts.
    guard !didStartStaging else { return }
    didStartStaging = true
    BuzzShareInbox.pruneStale(appGroupIdentifier: appGroupIdentifier)
    Task { await stageAndCompose() }
  }

  private func stageAndCompose() async {
    let providers = (extensionContext?.inputItems as? [NSExtensionItem])?
      .flatMap { $0.attachments ?? [] } ?? []
    guard !providers.isEmpty else {
      cancel(message: "Nothing to share")
      return
    }
    do {
      let (id, directory) = try BuzzShareInbox.makePayloadDirectory(
        appGroupIdentifier: appGroupIdentifier
      )
      var items: [BuzzShareInbox.Item] = []
      for provider in providers.prefix(BuzzShareInbox.maximumItems) {
        if let item = await ShareItemLoader.load(provider, into: directory) {
          items.append(item)
        }
      }
      guard !items.isEmpty else {
        try? FileManager.default.removeItem(at: directory)
        cancel(message: "Unsupported content")
        return
      }
      stagedID = id
      stagedDirectory = directory
      stagedItems = items
      showComposer()
    } catch {
      cancel(message: "Could not share to Buzz")
    }
  }

  private func showComposer() {
    let preview = SharePreview(
      text: stagedItems
        .filter { $0.kind != .file }
        .compactMap(\.value)
        .joined(separator: "\n"),
      fileNames: stagedItems.compactMap { $0.kind == .file ? ($0.name ?? "file") : nil },
      thumbnails: stagedItems.compactMap { item in
        guard item.kind == .file, item.mimeType?.hasPrefix("image/") == true,
          let path = item.path
        else { return nil }
        return Self.thumbnail(atPath: path, maxPixelSize: 168)
      }
    )
    let targets = BuzzShareTargets.read(appGroupIdentifier: appGroupIdentifier)
    let composer = ShareComposerView(
      preview: preview,
      targets: targets,
      onSend: { [weak self] target, message in
        self?.handOff(
          target: BuzzShareInbox.Target(communityID: target.communityID, channelID: target.channelID),
          message: message
        )
      },
      onContinueInApp: { [weak self] message in
        self?.handOff(target: nil, message: message)
      },
      onCancel: { [weak self] in
        self?.discardStaged()
        self?.cancel(message: "")
      }
    )
    let host = UIHostingController(rootView: composer)
    addChild(host)
    host.view.translatesAutoresizingMaskIntoConstraints = false
    view.addSubview(host.view)
    NSLayoutConstraint.activate([
      host.view.leadingAnchor.constraint(equalTo: view.leadingAnchor),
      host.view.trailingAnchor.constraint(equalTo: view.trailingAnchor),
      host.view.topAnchor.constraint(equalTo: view.topAnchor),
      host.view.bottomAnchor.constraint(equalTo: view.bottomAnchor),
    ])
    host.didMove(toParent: self)
    statusLabel.isHidden = true
  }

  /// Writes the manifest with the user's edited message and destination, then
  /// opens the app. Files stay as staged; text items are replaced by the
  /// message so edits made in the sheet win.
  private func handOff(target: BuzzShareInbox.Target?, message: String) {
    guard let id = stagedID, let directory = stagedDirectory else { return }
    let trimmed = message.trimmingCharacters(in: .whitespacesAndNewlines)
    var items = stagedItems.filter { $0.kind == .file }
    if !trimmed.isEmpty {
      items.insert(
        BuzzShareInbox.Item(kind: .text, value: trimmed, path: nil, name: nil, mimeType: nil),
        at: 0
      )
    }
    let payload = BuzzShareInbox.Payload(
      id: id,
      createdAt: Date().timeIntervalSince1970,
      items: items,
      target: target
    )
    do {
      try BuzzShareInbox.writeManifest(payload, in: directory)
    } catch {
      cancel(message: "Could not share to Buzz")
      return
    }
    openContainerApp(payloadID: id)
  }

  /// Decodes a small thumbnail without loading the full image, keeping several
  /// large photos well inside the extension's memory limit.
  private static func thumbnail(atPath path: String, maxPixelSize: Int) -> UIImage? {
    let url = URL(fileURLWithPath: path) as CFURL
    guard let source = CGImageSourceCreateWithURL(url, nil) else { return nil }
    let options: [CFString: Any] = [
      kCGImageSourceCreateThumbnailFromImageAlways: true,
      kCGImageSourceCreateThumbnailWithTransform: true,
      kCGImageSourceThumbnailMaxPixelSize: maxPixelSize,
    ]
    guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary)
    else { return nil }
    return UIImage(cgImage: image)
  }

  private func discardStaged() {
    if let directory = stagedDirectory {
      try? FileManager.default.removeItem(at: directory)
    }
  }

  private func openContainerApp(payloadID: String) {
    var components = URLComponents()
    components.scheme = "buzz"
    components.host = "share"
    components.queryItems = [URLQueryItem(name: "id", value: payloadID)]
    guard let url = components.url else {
      finishSaved()
      return
    }
    // No supported API lets a share extension open its containing app:
    // `NSExtensionContext.open` is documented for Today and iMessage
    // extensions and returns false here. Fall back to asking the application
    // object in the responder chain. If neither confirms, the payload is
    // already saved and the app picks it up on its next foreground, so tell
    // the user instead of closing silently.
    extensionContext?.open(url) { [weak self] opened in
      shareLog.info("extensionContext.open returned \(opened, privacy: .public)")
      DispatchQueue.main.async {
        guard let self else { return }
        if opened {
          self.finish()
          return
        }
        self.openViaResponderChain(url) { [weak self] success in
          DispatchQueue.main.async {
            if success { self?.finish() } else { self?.finishSaved() }
          }
        }
      }
    }
  }

  /// Asks the `UIApplication` instance at the end of the responder chain to
  /// open [url]. `openURL:options:completionHandler:` is public API but marked
  /// unavailable to extensions, so it is looked up by selector. Only the
  /// application object is asked: the hosted `UIScene` earlier in an
  /// extension's chain also answers the selector but aborts when called.
  private func openViaResponderChain(_ url: URL, completion: @escaping (Bool) -> Void) {
    let selector = NSSelectorFromString("openURL:options:completionHandler:")
    guard let applicationClass = NSClassFromString("UIApplication") else {
      completion(false)
      return
    }
    var responder: UIResponder? = self
    while let current = responder {
      if current.isKind(of: applicationClass), current.responds(to: selector),
        let implementation = class_getMethodImplementation(type(of: current), selector)
      {
        typealias OpenFunction = @convention(c) (
          AnyObject, Selector, URL, NSDictionary, (@convention(block) (Bool) -> Void)?
        ) -> Void
        let open = unsafeBitCast(implementation, to: OpenFunction.self)
        open(current, selector, url, [:]) { success in
          shareLog.info("open:options:completionHandler: success=\(success, privacy: .public)")
          completion(success)
        }
        return
      }
      responder = current.next
    }
    shareLog.error("no responder able to open the container app")
    completion(false)
  }

  /// The share is staged but Buzz could not be opened: say so, then close.
  private func finishSaved() {
    children.forEach { child in
      child.willMove(toParent: nil)
      child.view.removeFromSuperview()
      child.removeFromParent()
    }
    statusLabel.isHidden = false
    statusLabel.numberOfLines = 0
    statusLabel.textAlignment = .center
    statusLabel.text = "Saved. Open Buzz to finish sharing."
    UIAccessibility.post(notification: .announcement, argument: statusLabel.text)
    DispatchQueue.main.asyncAfter(deadline: .now() + 1.8) { [weak self] in
      self?.finish()
    }
  }

  private func cancel(message: String) {
    statusLabel.isHidden = false
    statusLabel.text = message
    let delay: TimeInterval = message.isEmpty ? 0 : 0.8
    DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self] in
      guard let self, !self.didFinish else { return }
      self.didFinish = true
      self.extensionContext?.cancelRequest(
        withError: NSError(domain: "xyz.block.buzz.share", code: 1)
      )
    }
  }

  private func finish() {
    guard !didFinish else { return }
    didFinish = true
    // Give the URL open a moment to reach the system before tearing down.
    DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) { [weak self] in
      self?.extensionContext?.completeRequest(returningItems: nil)
    }
  }
}

/// Resolves one `NSItemProvider` into a staged inbox item.
enum ShareItemLoader {
  static func load(
    _ provider: NSItemProvider,
    into directory: URL
  ) async -> BuzzShareInbox.Item? {
    // Order matters: a photo from Photos also conforms to public.url on some
    // hosts, and a page shared from Safari carries plain text alongside the URL.
    if provider.hasItemConformingToTypeIdentifier(UTType.image.identifier) {
      return await loadFile(provider, type: .image, into: directory)
    }
    if provider.hasItemConformingToTypeIdentifier(UTType.movie.identifier) {
      return await loadFile(provider, type: .movie, into: directory)
    }
    if provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier) {
      return await loadFileURL(provider, into: directory)
    }
    if provider.hasItemConformingToTypeIdentifier(UTType.url.identifier) {
      if let url = await loadObject(provider, type: .url) as? URL {
        if url.isFileURL {
          return copyFile(at: url, into: directory)
        }
        return BuzzShareInbox.Item(
          kind: .url, value: url.absoluteString, path: nil, name: nil, mimeType: nil
        )
      }
    }
    if provider.hasItemConformingToTypeIdentifier(UTType.plainText.identifier) {
      if let text = await loadObject(provider, type: .plainText) as? String,
        !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
      {
        return BuzzShareInbox.Item(
          kind: .text, value: text, path: nil, name: nil, mimeType: nil
        )
      }
    }
    if let type = provider.registeredTypeIdentifiers.compactMap(UTType.init).first(where: {
      $0.conforms(to: .data)
    }) {
      return await loadFile(provider, type: type, into: directory)
    }
    return nil
  }

  private static func loadObject(_ provider: NSItemProvider, type: UTType) async -> Any? {
    await withCheckedContinuation { continuation in
      provider.loadItem(forTypeIdentifier: type.identifier, options: nil) { item, _ in
        continuation.resume(returning: item)
      }
    }
  }

  private static func loadFileURL(
    _ provider: NSItemProvider,
    into directory: URL
  ) async -> BuzzShareInbox.Item? {
    let item = await loadObject(provider, type: .fileURL)
    let url: URL?
    if let direct = item as? URL {
      url = direct
    } else if let data = item as? Data {
      url = URL(dataRepresentation: data, relativeTo: nil)
    } else {
      url = nil
    }
    guard let url else { return nil }
    return copyFile(at: url, into: directory)
  }

  private static func loadFile(
    _ provider: NSItemProvider,
    type: UTType,
    into directory: URL
  ) async -> BuzzShareInbox.Item? {
    await withCheckedContinuation { continuation in
      provider.loadFileRepresentation(forTypeIdentifier: type.identifier) { url, _ in
        // The URL is only valid inside this closure; copy synchronously.
        guard let url else {
          continuation.resume(returning: nil)
          return
        }
        continuation.resume(returning: copyFile(at: url, into: directory, fallbackType: type))
      }
    }
  }

  private static func copyFile(
    at source: URL,
    into directory: URL,
    fallbackType: UTType? = nil
  ) -> BuzzShareInbox.Item? {
    let type = UTType(filenameExtension: source.pathExtension) ?? fallbackType
    var name = sanitizedFileName(source.lastPathComponent)
    if name.isEmpty { name = "shared" }
    if (name as NSString).pathExtension.isEmpty, let ext = type?.preferredFilenameExtension {
      name += ".\(ext)"
    }
    var destination = directory.appendingPathComponent(name)
    var suffix = 1
    while FileManager.default.fileExists(atPath: destination.path) {
      let base = (name as NSString).deletingPathExtension
      let ext = (name as NSString).pathExtension
      let candidate = ext.isEmpty ? "\(base)-\(suffix)" : "\(base)-\(suffix).\(ext)"
      destination = directory.appendingPathComponent(candidate)
      suffix += 1
    }
    do {
      try FileManager.default.copyItem(at: source, to: destination)
    } catch {
      return nil
    }
    return BuzzShareInbox.Item(
      kind: .file,
      value: nil,
      path: destination.path,
      name: destination.lastPathComponent,
      mimeType: type?.preferredMIMEType
    )
  }

  private static func sanitizedFileName(_ raw: String) -> String {
    let allowed = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "._-"))
    return String(raw.unicodeScalars.map { allowed.contains($0) ? Character($0) : "_" })
  }
}
