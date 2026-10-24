import Flutter
import UIKit

@main
struct MeAgentAccessibilityTests {
  @MainActor static func main() {
    for locked in [false, true] {
      for empty in [false, true] {
        let controller = NativeProfileTextEditorViewController(
          title: "New chat", initialValue: "Chat", placeholder: "Chat name",
          multiline: false, pageBackgroundColor: .systemGroupedBackground,
          containerBackgroundColor: .secondarySystemGroupedBackground,
          containerCornerRadius: 16, allowUnchangedSubmission: false,
          channelDescription: nil, originalName: nil, originalDescription: nil,
          canEditDetails: true, canEditCanvas: false, canvasContent: "",
          canvasLoaded: false,
          chatOptions: ["fieldsLocked": locked, "assistants": empty ? [] : [["id": "agent", "name": "Agent"]]],
          onCancel: {}, onSet: { _ in }
        )
        controller.loadViewIfNeeded()
        precondition(controller.numberOfSections(in: controller.tableView) == 3)
        let projectCell = controller.tableView(controller.tableView, cellForRowAt: IndexPath(row: 0, section: 1))
        let project = projectCell.accessoryView as! UIButton
        precondition(project.accessibilityLabel == "Project")
        precondition(project.accessibilityValue == "No project")
        precondition(project.titleColor(for: .normal) == .label)
        precondition(project.isEnabled == !locked)
        precondition(project.showsMenuAsPrimaryAction && project.menu != nil)
        let cell = controller.tableView(controller.tableView, cellForRowAt: IndexPath(row: 0, section: 2))
        let disabled = locked || empty
        precondition(cell.accessibilityTraits.contains(.notEnabled) == disabled)
        precondition(cell.accessibilityTraits.contains(.button) == !disabled)
        precondition((cell.selectionStyle == .none) == disabled)
      }
    }
    avatarTests()
    print("Me agent accessibility and bounded avatar loading passed")
  }
}

private final class AvatarProtocol: URLProtocol {
  static let lock = NSLock()
  static var starts = 0
  static var stops = 0
  static var png = Data()
  static func counts() -> (Int, Int) {
    lock.lock(); defer { lock.unlock() }
    return (starts, stops)
  }
  override class func canInit(with request: URLRequest) -> Bool { true }
  override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
  override func startLoading() {
    Self.lock.lock(); Self.starts += 1; Self.lock.unlock()
    let path = request.url!.path
    if path.hasPrefix("/hold") { return }
    let headers = ["Content-Type": path == "/text" ? "text/html" : "image/png",
                   "Content-Length": path == "/large" ? "9000000" : "-1"]
    client?.urlProtocol(self, didReceive: HTTPURLResponse(url: request.url!, statusCode: path == "/error" ? 404 : 200,
      httpVersion: nil, headerFields: headers)!, cacheStoragePolicy: .notAllowed)
    if path == "/chunked" {
      for _ in 0..<3 { client?.urlProtocol(self, didLoad: Data(repeating: 0, count: 1024 * 1024)) }
    } else {
      client?.urlProtocol(self, didLoad: Self.png)
    }
    client?.urlProtocolDidFinishLoading(self)
  }
  override func stopLoading() {
    Self.lock.lock(); Self.stops += 1; Self.lock.unlock()
  }
}

@MainActor private func avatarTests() {
  func pump(until predicate: () -> Bool) {
    let deadline = Date().addingTimeInterval(4)
    while !predicate() && Date() < deadline {
      RunLoop.main.run(until: Date().addingTimeInterval(0.01))
    }
    precondition(predicate(), "Avatar operation timed out")
  }
  let config = URLSessionConfiguration.ephemeral
  config.protocolClasses = [AvatarProtocol.self]
  let loader = MeAvatarLoader(configuration: config)
  let format = UIGraphicsImageRendererFormat()
  format.scale = 1
  AvatarProtocol.png = UIGraphicsImageRenderer(size: CGSize(width: 240, height: 240), format: format).pngData { _ in
    UIColor.red.setFill(); UIRectFill(CGRect(x: 0, y: 0, width: 240, height: 240))
  }
  // No eager request on presenting even a thousand agents; only visible rows load.
  let controller = MeAssistantListController(assistants: (0..<1000).map {
    ["id": "agent-\($0)", "name": "Agent \($0)", "avatarUrl": "https://avatar.test/hold/\($0)"]
  }, selected: nil, pageColor: .white, rowColor: .white, avatars: loader, onSelect: { _, _ in })
  controller.loadViewIfNeeded()
  precondition(AvatarProtocol.counts().0 == 0)
  for row in 0..<20 {
    let path = IndexPath(row: row, section: 0)
    let cell = controller.tableView(controller.tableView, cellForRowAt: path)
    controller.tableView(controller.tableView, willDisplay: cell, forRowAt: path)
  }
  pump { AvatarProtocol.counts().0 == MeAvatarLoader.concurrency }
  controller.viewDidDisappear(false)
  pump { AvatarProtocol.counts().1 == MeAvatarLoader.concurrency }
  precondition(AvatarProtocol.counts().0 == MeAvatarLoader.concurrency)

  for path in ["large", "chunked", "error", "text", "ok"] {
    var done = false
    var result: UIImage?
    loader.load(URL(string: "https://avatar.test/\(path)")!) { image in
      result = image; done = true
    }
    pump { done }
    precondition((result != nil) == (path == "ok"))
    if let result { precondition(max(result.size.width, result.size.height) <= 120) }
  }
  let image = loader.image(for: URL(string: "https://avatar.test/ok"))
  precondition(image != nil)
  let rendered = MeAssistantListController.avatarImage(["name": "Agent"], photo: image)
  precondition(rendered.size == CGSize(width: 40, height: 40))
  let visibleLoader = MeAvatarLoader(configuration: config)
  let visible = MeAssistantListController(assistants: [["id": "visible", "name": "Visible", "avatarUrl": "https://avatar.test/visible"]],
    selected: nil, pageColor: .white, rowColor: .white, avatars: visibleLoader, onSelect: { _, _ in })
  visible.loadViewIfNeeded()
  visible.tableView.frame = CGRect(x: 0, y: 0, width: 320, height: 400)
  visible.tableView.reloadData()
  visible.tableView.layoutIfNeeded()
  pump { visibleLoader.image(for: URL(string: "https://avatar.test/visible")) != nil }
  let cell = visible.tableView.cellForRow(at: IndexPath(row: 0, section: 0))!
  let content = cell.contentConfiguration as! UIListContentConfiguration
  precondition(content.image?.pngData() == MeAssistantListController.avatarImage(["name": "Visible"], photo: image).pngData())
  visible.viewDidDisappear(false)
  // Source metadata is inspected before ImageIO is allowed to decode large pixels.
  autoreleasepool {
    let huge = UIGraphicsImageRenderer(size: CGSize(width: 4100, height: 4100), format: format).pngData { _ in }
    precondition(MeAvatarLoader.thumbnail(huge) == nil)
  }
  loader.cancelAll()
}
