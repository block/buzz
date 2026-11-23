import BuzzPushKit
import Flutter
import Intents
import UIKit
import UniformTypeIdentifiers

/// Prototype: shows Buzz's Flutter share screen inside the extension and sends
/// from this process, like a separate device that shares only the identity
/// (keychain and app group). Picking "New Solo chat" still hands the share to
/// the app, as before.
final class ShareViewController: UIViewController {
  private var engine: FlutterEngine?
  private var channel: FlutterMethodChannel?
  private lazy var content = Task { await SharedContent.load(from: self.extensionContext) }
  private var relay: ShareRelayClient?

  override func viewDidLoad() {
    super.viewDidLoad()
    view.backgroundColor = .systemBackground
    _ = content
    startFlutter()
  }

  private func startFlutter() {
    // The extension runs the containing app's Dart code and engine, so
    // nothing is duplicated in the extension bundle.
    let appURL = Bundle.main.bundleURL.deletingLastPathComponent().deletingLastPathComponent()
    let dartBundle = Bundle(url: appURL.appendingPathComponent("Frameworks/App.framework"))
    let project = FlutterDartProject(precompiledDartBundle: dartBundle)
    let engine = FlutterEngine(name: "share", project: project, allowHeadlessExecution: false)
    engine.run(
      withEntrypoint: "shareExtensionMain",
      libraryURI: "package:buzz/share_extension/share_extension_main.dart")
    let channel = FlutterMethodChannel(name: "buzz/share_extension", binaryMessenger: engine.binaryMessenger)
    channel.setMethodCallHandler { [weak self] call, result in
      guard let self else { return result(nil) }
      Task { @MainActor in
        do { result(try await self.handle(call.method, call.arguments)) }
        catch { result(FlutterError(code: "failed", message: ShareError.describe(error), details: nil)) }
      }
    }
    let controller = FlutterViewController(engine: engine, nibName: nil, bundle: nil)
    addChild(controller)
    controller.view.frame = view.bounds
    controller.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
    view.addSubview(controller.view)
    controller.didMove(toParent: self)
    self.engine = engine
    self.channel = channel
  }

  @MainActor
  private func handle(_ method: String, _ arguments: Any?) async throws -> Any? {
    let args = arguments as? [String: Any] ?? [:]
    switch method {
    case "load":
      let shared = await content.value
      var result: [String: Any] = ["text": shared.text, "missed": shared.missed]
      if let image = shared.image {
        result["image"] = ["path": image.url.path, "mime": image.mime, "size": image.size]
      }
      if let problem = shared.problem { result["problem"] = problem }
      let target = (extensionContext?.intent as? INSendMessageIntent)?.conversationIdentifier
      if let target { result["suggested"] = target }
      do {
        let relay = try ShareRelayClient.current()
        self.relay = relay
        result["community"] = [
          "id": relay.community.id, "name": relay.community.name, "pubkey": relay.pubkey,
          "relay": relay.baseURL.absoluteString,
        ]
        result["bestie"] = Self.bestie(pubkey: relay.pubkey)
      } catch {
        result["signedOut"] = ShareError.describe(error)
      }
      return result
    case "query":
      guard let relay else { throw ShareError.notSignedIn }
      let filters = args["filters"] as? [[String: Any]] ?? []
      return try await relay.query(filters)
    case "upload":
      guard let relay else { throw ShareError.notSignedIn }
      guard let image = await content.value.image else { throw ShareError.message("No image to send.") }
      return try await relay.upload(image)
    case "publish":
      guard let relay else { throw ShareError.notSignedIn }
      return try await relay.publish(
        kind: args["kind"] as? Int ?? 9, content: args["content"] as? String ?? "",
        tags: args["tags"] as? [[String]] ?? [],
        createdAt: args["createdAt"] as? Int ?? Int(Date().timeIntervalSince1970))
    case "close":
      extensionContext?.completeRequest(returningItems: nil)
      return nil
    case "cancel":
      extensionContext?.cancelRequest(withError: NSError(domain: NSCocoaErrorDomain, code: NSUserCancelledError))
      return nil
    case "openApp":
      // A new Solo chat needs the app's creation flow, so hand the share over.
      try await handOff(note: args["note"] as? String)
      return nil
    default:
      return FlutterMethodNotImplemented
    }
  }

  /// The Bestie the app last published for this identity, if any.
  private static func bestie(pubkey: String) -> [String: String]? {
    guard let url = ShareRelayClient.appGroupURL?.appendingPathComponent("Library/Caches/share-bestie.json"),
      let data = try? Data(contentsOf: url),
      let json = try? JSONSerialization.jsonObject(with: data) as? [String: String],
      json["pubkey"] == pubkey, let bestie = json["bestie"]
    else { return nil }
    return ["pubkey": bestie, "name": json["name"] ?? ""]
  }

  private func handOff(note: String?) async throws {
    let shared = await content.value
    let shareID = UUID().uuidString
    guard let root = ShareRelayClient.appGroupURL else { throw ShareError.message("The app group is unavailable.") }
    let directory = root.appendingPathComponent("PendingShares/\(shareID)", isDirectory: true)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    var files: [String] = []
    if let image = shared.image {
      let name = "\(UUID().uuidString).\(image.url.pathExtension)"
      try FileManager.default.copyItem(at: image.url, to: directory.appendingPathComponent(name))
      files.append(name)
    }
    let text = note.map { $0.isEmpty ? [] : [$0] } ?? shared.text
    let manifest: [String: Any] = ["version": 1, "text": text, "files": files, "missed": shared.missed]
    try JSONSerialization.data(withJSONObject: manifest)
      .write(to: directory.appendingPathComponent("share.json"), options: .atomic)
    var components = URLComponents()
    components.scheme = Bundle.main.object(forInfoDictionaryKey: "BuzzShareURLScheme") as? String
    components.host = "share"
    components.queryItems = [URLQueryItem(name: "id", value: shareID)]
    if !openContainingApp(components.url!) {
      NSLog("[BuzzShare] no UIApplication in the responder chain; share %@ stays pending", shareID)
    }
    extensionContext?.completeRequest(returningItems: nil)
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

enum ShareError: Error {
  case notSignedIn
  case message(String)

  static func describe(_ error: Error) -> String {
    switch error {
    case ShareError.notSignedIn: return "Sign in to Buzz and turn on notifications to share from here."
    case ShareError.message(let text): return text
    default: return error.localizedDescription
    }
  }
}

/// What the person shared: text and links, and at most one photo.
struct SharedContent {
  struct Image {
    let url: URL
    let mime: String
    let size: Int
  }

  var text: [String] = []
  var image: Image?
  var missed = 0
  var problem: String?

  /// The relay's default image limit.
  static let maximumImageBytes = 50 * 1024 * 1024

  static func load(from context: NSExtensionContext?) async -> SharedContent {
    var shared = SharedContent()
    let providers = ((context?.inputItems as? [NSExtensionItem]) ?? []).flatMap { $0.attachments ?? [] }
    for provider in providers {
      NSLog("[BuzzShare] offered types: %@", provider.registeredTypeIdentifiers.joined(separator: ", "))
      if provider.hasItemConformingToTypeIdentifier(UTType.image.identifier) {
        guard shared.image == nil else {
          shared.missed += 1
          continue
        }
        do { shared.image = try await ShareImage.prepare(provider) }
        catch {
          shared.missed += 1
          shared.problem = ShareError.describe(error)
        }
        continue
      }
      if provider.hasItemConformingToTypeIdentifier(UTType.movie.identifier) {
        shared.missed += 1
        shared.problem = "Videos aren't supported in this prototype yet."
        continue
      }
      var loaded = false
      for type in [UTType.url, .plainText] where provider.hasItemConformingToTypeIdentifier(type.identifier) {
        let item = try? await provider.loadItem(forTypeIdentifier: type.identifier)
        if let url = item as? URL { shared.text.append(url.absoluteString); loaded = true }
        else if let string = item as? String { shared.text.append(string); loaded = true }
        break
      }
      if !loaded { shared.missed += 1 }
    }
    return shared
  }
}
