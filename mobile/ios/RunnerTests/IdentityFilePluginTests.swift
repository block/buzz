import Flutter
import UIKit
import XCTest
@testable import Buzz

/// Actual UIKit picker presentation and method-channel dispatch, followed by
/// synthetic document-provider delegate callbacks. Never persists an identity.
final class IdentityFilePluginTests: XCTestCase {
  @MainActor
  private func makeWindow() throws -> (UIWindow, UIViewController) {
    let scene = try XCTUnwrap(UIApplication.shared.connectedScenes
      .compactMap { $0 as? UIWindowScene }
      .first { $0.activationState == .foregroundActive })
    let parent = UIViewController()
    let window = UIWindow(windowScene: scene)
    window.rootViewController = parent
    window.makeKeyAndVisible()
    parent.loadViewIfNeeded()
    return (window, parent)
  }

  @MainActor
  private func picker(in parent: UIViewController) async throws -> UIDocumentPickerViewController {
    let presented = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
      parent.presentedViewController is UIDocumentPickerViewController
    }, object: nil)
    await fulfillment(of: [presented], timeout: 5)
    return try XCTUnwrap(parent.presentedViewController as? UIDocumentPickerViewController)
  }

  @MainActor
  private func settle(_ controller: UIViewController) async {
    if let transition = controller.transitionCoordinator {
      await withCheckedContinuation { continuation in
        transition.animate(alongsideTransition: nil) { _ in continuation.resume() }
      }
    }
  }

  @MainActor
  private func dismiss(_ parent: UIViewController) async {
    await withCheckedContinuation { continuation in
      parent.dismiss(animated: false) { continuation.resume() }
    }
  }

  @MainActor
  func testPickerResolvesPresenterAtRequestAndCancelsExactlyOnce() async throws {
    let messenger = IdentityFileTestMessenger()
    // Initialize before this window exists: registrar startup timing must not
    // permanently prevent later user-triggered presentation.
    let plugin = IdentityFilePlugin(messenger: messenger)
    let (window, parent) = try makeWindow()
    defer { window.isHidden = true }
    var responses = 0
    messenger.invokeImport { value in
      responses += 1
      XCTAssertNil(value)
    }
    let picker = try await picker(in: parent)
    await settle(picker)
    XCTAssertFalse(picker.allowsMultipleSelection)
    XCTAssertEqual(responses, 0)
    var busy: FlutterError?
    messenger.invokeImport { busy = $0 as? FlutterError }
    XCTAssertEqual(busy?.code, "import_busy")
    XCTAssertNil(busy?.details)
    let attachment = XCTAttachment(image: UIGraphicsImageRenderer(bounds: window.bounds).image { _ in
      window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
    })
    attachment.name = "Identity document picker presented"
    attachment.lifetime = .keepAlways
    add(attachment)
    plugin.documentPickerWasCancelled(picker)
    plugin.documentPickerWasCancelled(picker)
    XCTAssertEqual(responses, 1)
    await dismiss(parent)
    messenger.invokeImport { value in
      responses += 1
      XCTAssertNil(value)
    }
    let next = try await self.picker(in: parent)
    await settle(next)
    // Late callbacks from the previous picker must not finish the new request.
    plugin.documentPickerWasCancelled(picker)
    XCTAssertEqual(responses, 1)
    plugin.documentPicker(picker, didPickDocumentsAt: [])
    XCTAssertEqual(responses, 1)
    plugin.documentPickerWasCancelled(next)
    XCTAssertEqual(responses, 2)
    await dismiss(parent)
  }

  @MainActor
  func testSyntheticDocumentsReadInPlaceAndRejectInvalidOrOversize() async throws {
    let messenger = IdentityFileTestMessenger()
    let plugin = IdentityFilePlugin(messenger: messenger)
    let (window, parent) = try makeWindow()
    defer { window.isHidden = true }
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let cases: [(Data, String?)] = [
      (Data("synthetic-nsec-file-not-an-identity\n".utf8), "synthetic-nsec-file-not-an-identity\n"),
      (Data(repeating: 97, count: 1024), String(repeating: "a", count: 1024)),
      (Data(repeating: 97, count: 1025), nil),
      (Data([0xc3, 0x28]), nil),
      (Data(), nil),
    ]
    for (index, entry) in cases.enumerated() {
      let url = directory.appendingPathComponent("fixture-\(index).nsec")
      try entry.0.write(to: url)
      let completed = expectation(description: "file result \(index)")
      messenger.invokeImport { value in
        if let expected = entry.1 { XCTAssertEqual(value as? String, expected) }
        else {
          let error = value as? FlutterError
          XCTAssertEqual(error?.code, "import_failed")
          XCTAssertNil(error?.message)
          XCTAssertNil(error?.details)
        }
        completed.fulfill()
      }
      let picker = try await picker(in: parent)
      await settle(picker)
      // A synthetic delegate URL, not a claim that Files/iCloud selected it.
      plugin.documentPicker(picker, didPickDocumentsAt: [url])
      await fulfillment(of: [completed], timeout: 5)
      XCTAssertEqual(try Data(contentsOf: url), entry.0, "Original document must remain untouched")
      await dismiss(parent)
    }
  }
}

private final class IdentityFileTestMessenger: NSObject, FlutterBinaryMessenger {
  private var handler: FlutterBinaryMessageHandler?
  func send(onChannel channel: String, message: Data?) {}
  func send(onChannel channel: String, message: Data?, binaryReply callback: FlutterBinaryReply?) { callback?(nil) }
  func setMessageHandlerOnChannel(_ channel: String, binaryMessageHandler handler: FlutterBinaryMessageHandler?) -> FlutterBinaryMessengerConnection {
    self.handler = handler
    return 1
  }
  func cleanUpConnection(_ connection: FlutterBinaryMessengerConnection) { handler = nil }
  func invokeImport(reply: @escaping (Any?) -> Void) {
    let codec = FlutterStandardMethodCodec.sharedInstance()
    handler?(codec.encode(FlutterMethodCall(methodName: "import", arguments: nil))) { data in
      reply(data.flatMap { codec.decodeEnvelope($0) })
    }
  }
}
