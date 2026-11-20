import Intents
import UIKit
import UniformTypeIdentifiers

/// Copies the shared items into the app group and opens Buzz on the chosen
/// conversation. The extension has no UI and sends nothing; the app reads the
/// pending share and prefills that conversation's composer.
final class ShareViewController: UIViewController {
  private var handled = false

  override func viewDidAppear(_ animated: Bool) {
    super.viewDidAppear(animated)
    guard !handled else { return }
    handled = true
    Task { await handOff() }
  }

  private func handOff() async {
    let shareID = UUID().uuidString
    let directory = PendingShareDirectory.create(shareID)
    let target = (extensionContext?.intent as? INSendMessageIntent)?.conversationIdentifier
    var text: [String] = []
    var files: [String] = []
    var documents: [[String: String]] = []
    // Items that couldn't be loaded, so the app can say some didn't arrive.
    var missed = 0
    let items = (extensionContext?.inputItems as? [NSExtensionItem]) ?? []
    for provider in items.flatMap({ $0.attachments ?? [] }) {
      switch await load(provider, into: directory) {
      case .text(let value): text.append(value)
      case .file(let name): files.append(name)
      case .document(let file, let name): documents.append(["file": file, "name": name])
      case nil: missed += 1
      }
    }
    var manifest: [String: Any] = [
      "version": 1, "text": text, "files": files, "documents": documents, "missed": missed,
    ]
    if let target { manifest["target"] = target }
    let data = try! JSONSerialization.data(withJSONObject: manifest)
    try! data.write(to: directory.appendingPathComponent("share.json"), options: .atomic)

    var components = URLComponents()
    components.scheme = Bundle.main.object(forInfoDictionaryKey: "BuzzShareURLScheme") as? String
    components.host = "share"
    components.queryItems = [URLQueryItem(name: "id", value: shareID)]
    if !openContainingApp(components.url!) {
      NSLog("[BuzzShare] no UIApplication in the responder chain; share %@ stays pending", shareID)
    }
    extensionContext?.completeRequest(returningItems: nil)
  }

  private enum Loaded {
    case text(String)
    case file(String)
    /// Any other file, stored as the first value and named by the second.
    case document(String, String)
  }

  private func load(_ provider: NSItemProvider, into directory: URL) async -> Loaded? {
    // The app tells videos from images by file extension, which the copy keeps.
    if let type = [UTType.movie, .image]
      .first(where: { provider.hasItemConformingToTypeIdentifier($0.identifier) })
    {
      return await copyFile(provider, type: type, into: directory)
    }
    // A file URL is a document (a PDF from Files, say), not a link to paste.
    if !provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier),
      let type = [UTType.url, .plainText]
        .first(where: { provider.hasItemConformingToTypeIdentifier($0.identifier) })
    {
      let item = try? await provider.loadItem(forTypeIdentifier: type.identifier)
      if let url = item as? URL { return .text(url.absoluteString) }
      if let string = item as? String { return .text(string) }
      NSLog("[BuzzShare] %@ item had unexpected type %@", type.identifier, String(describing: item.map { Swift.type(of: $0) }))
      return nil
    }
    if let type = provider.registeredTypeIdentifiers.lazy.compactMap(UTType.init)
      .first(where: { $0.conforms(to: .data) && $0 != .fileURL })
    {
      return await copyDocument(provider, type: type, into: directory)
    }
    NSLog("[BuzzShare] skipped unsupported item %@", provider.registeredTypeIdentifiers)
    return nil
  }

  private func copyDocument(_ provider: NSItemProvider, type: UTType, into directory: URL) async -> Loaded? {
    await withCheckedContinuation { continuation in
      provider.loadFileRepresentation(forTypeIdentifier: type.identifier) { url, error in
        guard let url else {
          NSLog("[BuzzShare] couldn't load %@: %@", type.identifier, String(describing: error))
          return continuation.resume(returning: nil)
        }
        // The copy gets a safe unique name; the original name rides along.
        let ext = url.pathExtension.isEmpty ? (type.preferredFilenameExtension ?? "") : url.pathExtension
        let file = ext.isEmpty ? UUID().uuidString : "\(UUID().uuidString).\(ext)"
        var name = provider.suggestedName ?? url.deletingPathExtension().lastPathComponent
        if !ext.isEmpty, (name as NSString).pathExtension.isEmpty { name += ".\(ext)" }
        do {
          try FileManager.default.copyItem(at: url, to: directory.appendingPathComponent(file))
          continuation.resume(returning: .document(file, name))
        } catch {
          NSLog("[BuzzShare] couldn't copy %@: %@", type.identifier, String(describing: error))
          continuation.resume(returning: nil)
        }
      }
    }
  }

  private func copyFile(_ provider: NSItemProvider, type: UTType, into directory: URL) async -> Loaded? {
    await withCheckedContinuation { continuation in
      provider.loadFileRepresentation(forTypeIdentifier: type.identifier) { url, error in
        guard let url else {
          NSLog("[BuzzShare] couldn't load %@: %@", type.identifier, String(describing: error))
          return continuation.resume(returning: nil)
        }
        let name = "\(UUID().uuidString).\(url.pathExtension)"
        do {
          try FileManager.default.copyItem(at: url, to: directory.appendingPathComponent(name))
          continuation.resume(returning: .file(name))
        } catch {
          NSLog("[BuzzShare] couldn't copy %@: %@", type.identifier, String(describing: error))
          continuation.resume(returning: nil)
        }
      }
    }
  }

  /// UIApplication.open is unavailable to extensions, so reach the host
  /// UIApplication through the responder chain.
  private func openContainingApp(_ url: URL) -> Bool {
    let selector = NSSelectorFromString("openURL:options:completionHandler:")
    var responder: UIResponder? = self
    while let current = responder {
      if let app = current as? UIApplication, app.responds(to: selector) {
        typealias Open = @convention(c) (AnyObject, Selector, NSURL, NSDictionary, AnyObject?) -> Void
        let open = unsafeBitCast(app.method(for: selector), to: Open.self)
        open(app, selector, url as NSURL, NSDictionary(), nil)
        return true
      }
      responder = current.next
    }
    return false
  }
}

enum PendingShareDirectory {
  static func create(_ shareID: String) -> URL {
    guard let group = Bundle.main.object(forInfoDictionaryKey: "BuzzAppGroupIdentifier") as? String,
      let root = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: group)
    else { preconditionFailure("ShareExtension needs BuzzAppGroupIdentifier and the app group entitlement") }
    let directory = root.appendingPathComponent("PendingShares", isDirectory: true)
      .appendingPathComponent(shareID, isDirectory: true)
    try! FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    return directory
  }
}
