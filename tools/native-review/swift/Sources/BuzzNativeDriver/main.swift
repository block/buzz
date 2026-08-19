import AppKit
import ApplicationServices
import AVFoundation
import CoreGraphics
import Foundation

struct Locator: Codable {
    let id: String?
    let role: String?
    let name: String?
}

struct Size: Codable {
    let width: Double
    let height: Double
}

struct SemanticNode: Codable {
    let id: String?
    let role: String?
    let name: String?
    let enabled: Bool
    let focused: Bool
    let frame: Rect
    let viewport: Size
}

struct ElementDescription: Codable {
    let locator: Locator
    let role: String?
    let name: String?
    let identifier: String?
    let enabled: Bool
    let focused: Bool
    let frame: Rect?
}

struct Rect: Codable {
    let x: Double
    let y: Double
    let width: Double
    let height: Double
}

struct AXNode: Codable {
    let role: String?
    let name: String?
    let identifier: String?
    let value: String?
    let enabled: Bool?
    let focused: Bool?
    let frame: Rect?
    let children: [AXNode]
    let truncated: Bool?
}

enum DriverError: Error, CustomStringConvertible {
    case message(String)
    var description: String {
        switch self { case .message(let value): return value }
    }
}

struct CaptureCadence {
    private(set) var frame: Int64 = 0

    var presentationTime: CMTime { CMTime(value: frame, timescale: 15) }

    mutating func advance() { frame += 1 }
}

@discardableResult
func captureTick(
    cadence: inout CaptureCadence,
    isReady: Bool,
    append: (CMTime) -> Bool
) -> Bool {
    let presentationTime = cadence.presentationTime
    defer { cadence.advance() }
    guard isReady else { return true }
    return append(presentationTime)
}

func targetIsFrontmost(pid: pid_t, frontmostPID: () -> pid_t? = {
    NSWorkspace.shared.frontmostApplication?.processIdentifier
}) -> Bool {
    frontmostPID() == pid
}

func requireTargetIsFrontmost(pid: pid_t, frontmostPID: () -> pid_t? = {
    NSWorkspace.shared.frontmostApplication?.processIdentifier
}) throws {
    guard targetIsFrontmost(pid: pid, frontmostPID: frontmostPID) else {
        throw DriverError.message("refusing global input because target pid \(pid) is not frontmost")
    }
}

func postGlobalEvent(_ event: CGEvent?, pid: pid_t, frontmostPID: () -> pid_t? = {
    NSWorkspace.shared.frontmostApplication?.processIdentifier
}, post: (CGEvent) -> Void = { $0.post(tap: .cghidEventTap) }) throws {
    try requireTargetIsFrontmost(pid: pid, frontmostPID: frontmostPID)
    guard let event else { throw DriverError.message("failed to create global input event") }
    post(event)
}

func pointerPath(from start: CGPoint, to target: CGPoint, within bounds: CGRect, steps: Int) -> [CGPoint] {
    let insetBounds = bounds.insetBy(dx: min(1, bounds.width / 4), dy: min(1, bounds.height / 4))
    let safeStart = CGPoint(
        x: min(max(start.x, insetBounds.minX), insetBounds.maxX),
        y: min(max(start.y, insetBounds.minY), insetBounds.maxY)
    )
    return (1...max(steps, 1)).map { index in
        let fraction = Double(index) / Double(max(steps, 1))
        return CGPoint(
            x: safeStart.x + (target.x - safeStart.x) * fraction,
            y: safeStart.y + (target.y - safeStart.y) * fraction
        )
    }
}

func requirePointInTargetWindow(
    _ point: CGPoint,
    pid: pid_t,
    targetBounds: (pid_t) throws -> CGRect = { try windowInfo(pid: $0).1 }
) throws {
    guard try targetBounds(pid).contains(point) else {
        throw DriverError.message("refusing global pointer input outside target pid \(pid) window")
    }
}

func postGlobalPointerEvent(
    _ event: CGEvent?,
    at point: CGPoint,
    pid: pid_t,
    frontmostPID: () -> pid_t? = { NSWorkspace.shared.frontmostApplication?.processIdentifier },
    targetBounds: (pid_t) throws -> CGRect = { try windowInfo(pid: $0).1 },
    post: (CGEvent) -> Void = { $0.post(tap: .cghidEventTap) }
) throws {
    try requireTargetIsFrontmost(pid: pid, frontmostPID: frontmostPID)
    try requirePointInTargetWindow(point, pid: pid, targetBounds: targetBounds)
    guard let event else { throw DriverError.message("failed to create global input event") }
    post(event)
}

func enableWebViewAccessibility(_ app: AXUIElement) {
    // WebKit does not materialize its remote accessibility tree for ordinary
    // automation clients until manual/enhanced accessibility is requested.
    // VoiceOver does this implicitly; the review harness must not require it.
    let enabled = kCFBooleanTrue as CFTypeRef
    for name in ["AXManualAccessibility", "AXEnhancedUserInterface"] {
        _ = AXUIElementSetAttributeValue(app, name as CFString, enabled)
    }
}

func attribute(_ element: AXUIElement, _ name: String) -> AnyObject? {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success else { return nil }
    return value
}

func stringAttribute(_ element: AXUIElement, _ names: [String]) -> String? {
    for name in names {
        if let value = attribute(element, name) as? String, !value.isEmpty { return value }
    }
    return nil
}

func boolAttribute(_ element: AXUIElement, _ name: String, default fallback: Bool = false) -> Bool {
    (attribute(element, name) as? Bool) ?? fallback
}

func frameAttribute(_ element: AXUIElement) -> Rect? {
    guard let positionObject = attribute(element, kAXPositionAttribute),
          let sizeObject = attribute(element, kAXSizeAttribute),
          CFGetTypeID(positionObject) == AXValueGetTypeID(),
          CFGetTypeID(sizeObject) == AXValueGetTypeID() else { return nil }
    let positionValue = positionObject as! AXValue
    let sizeValue = sizeObject as! AXValue
    var point = CGPoint.zero
    var size = CGSize.zero
    guard AXValueGetValue(positionValue, .cgPoint, &point), AXValueGetValue(sizeValue, .cgSize, &size) else { return nil }
    return Rect(x: point.x, y: point.y, width: size.width, height: size.height)
}

func normalizedRole(_ raw: String?) -> String? {
    guard var value = raw else { return nil }
    if value.hasPrefix("AX") { value.removeFirst(2) }
    switch value.lowercased() {
    case "textarea", "text area": return "text-area"
    default: return value.lowercased()
    }
}

func elementName(_ element: AXUIElement) -> String? {
    stringAttribute(element, [kAXTitleAttribute, kAXDescriptionAttribute, kAXHelpAttribute, kAXValueAttribute])
}

func elementIdentifier(_ element: AXUIElement) -> String? {
    stringAttribute(element, [kAXIdentifierAttribute])
}

func elementRole(_ element: AXUIElement) -> String? {
    stringAttribute(element, [kAXRoleAttribute])
}

func matches(_ element: AXUIElement, locator: Locator) -> Bool {
    if let id = locator.id, elementIdentifier(element) != id { return false }
    if let role = locator.role, normalizedRole(elementRole(element)) != normalizedRole(role) { return false }
    if let name = locator.name, elementName(element) != name { return false }
    return true
}

func children(_ element: AXUIElement) -> [AXUIElement] {
    let direct = (attribute(element, kAXChildrenAttribute) as? [AXUIElement]) ?? []
    let windows = (attribute(element, kAXWindowsAttribute) as? [AXUIElement]) ?? []
    let singular = [kAXMainWindowAttribute, kAXFocusedWindowAttribute].compactMap {
        attribute(element, $0) as! AXUIElement?
    }
    var seen = Set<CFHashCode>()
    return (direct + windows + singular).filter { child in
        guard !CFEqual(child, element) else { return false }
        return seen.insert(CFHash(child)).inserted
    }
}

func accessibilityRoots(app: AXUIElement, pid: pid_t) -> [AXUIElement] {
    // WKWebView's remote tree is not always attached to AXChildren/AXWindows,
    // even after manual accessibility is enabled. Hit-testing the visible app
    // window asks the accessibility server for the element actually painted at
    // each point and reliably materializes the WebKit subtree without DOM IPC.
    var roots = [app]
    guard let (_, bounds) = try? windowInfo(pid: pid) else { return roots }
    let system = AXUIElementCreateSystemWide()
    for xFraction in stride(from: 0.05, through: 0.95, by: 0.1) {
        for yFraction in stride(from: 0.05, through: 0.95, by: 0.1) {
            var element: AXUIElement?
            let x = Float(bounds.minX + bounds.width * xFraction)
            let y = Float(bounds.minY + bounds.height * yFraction)
            if AXUIElementCopyElementAtPosition(system, x, y, &element) == .success,
               let element {
                roots.append(element)
            }
        }
    }
    var seen = Set<CFHashCode>()
    return roots.filter { seen.insert(CFHash($0)).inserted }
}

func find(_ roots: [AXUIElement], locator: Locator, maxNodes: Int = 20_000) -> AXUIElement? {
    var queue = roots
    var seen = Set<CFHashCode>()
    var visited = 0
    while !queue.isEmpty && visited < maxNodes {
        let current = queue.removeFirst()
        guard seen.insert(CFHash(current)).inserted else { continue }
        visited += 1
        if matches(current, locator: locator) { return current }
        queue.append(contentsOf: children(current))
    }
    return nil
}

func describe(_ element: AXUIElement, locator: Locator) -> ElementDescription {
    ElementDescription(locator: locator, role: normalizedRole(elementRole(element)), name: elementName(element),
                       identifier: elementIdentifier(element), enabled: boolAttribute(element, kAXEnabledAttribute, default: true),
                       focused: boolAttribute(element, kAXFocusedAttribute), frame: frameAttribute(element))
}

func semanticNodes(path: String, pid: pid_t) -> [SemanticNode] {
    guard let data = FileManager.default.contents(atPath: path),
          let nodes = try? JSONDecoder().decode([SemanticNode].self, from: data) else { return [] }
    guard let (_, windowBounds) = try? windowInfo(pid: pid) else { return nodes }
    return nodes.map { node in
        guard node.viewport.width > 0, node.viewport.height > 0 else { return node }
        let xScale = windowBounds.width / node.viewport.width
        let yScale = windowBounds.height / node.viewport.height
        return SemanticNode(
            id: node.id,
            role: node.role,
            name: node.name,
            enabled: node.enabled,
            focused: node.focused,
            frame: Rect(
                x: windowBounds.minX + node.frame.x * xScale,
                y: windowBounds.minY + node.frame.y * yScale,
                width: node.frame.width * xScale,
                height: node.frame.height * yScale
            ),
            viewport: node.viewport
        )
    }
}

func matches(_ element: SemanticNode, locator: Locator) -> Bool {
    if let id = locator.id, element.id != id { return false }
    if let role = locator.role, normalizedRole(element.role) != normalizedRole(role) { return false }
    if let name = locator.name, element.name != name { return false }
    return true
}

func describe(_ element: SemanticNode, locator: Locator) -> ElementDescription {
    ElementDescription(locator: locator, role: normalizedRole(element.role), name: element.name,
                       identifier: element.id, enabled: element.enabled, focused: element.focused,
                       frame: element.frame)
}

func snapshot(_ element: AXUIElement, depth: Int = 0, budget: inout Int) -> AXNode {
    budget -= 1
    let isTruncated = budget <= 0 || depth >= 80
    let descendants = isTruncated ? [] : children(element).map { snapshot($0, depth: depth + 1, budget: &budget) }
    return AXNode(role: normalizedRole(elementRole(element)), name: elementName(element), identifier: elementIdentifier(element),
                  value: stringAttribute(element, [kAXValueAttribute]), enabled: attribute(element, kAXEnabledAttribute) as? Bool,
                  focused: attribute(element, kAXFocusedAttribute) as? Bool, frame: frameAttribute(element),
                  children: descendants, truncated: isTruncated ? true : nil)
}

func windowInfo(pid: pid_t) throws -> (CGWindowID, CGRect) {
    guard let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] else {
        throw DriverError.message("cannot enumerate windows; Screen Recording permission may be missing")
    }
    let candidates = windows.compactMap { window -> (CGWindowID, CGRect)? in
        guard (window[kCGWindowOwnerPID as String] as? Int32) == pid,
              let number = window[kCGWindowNumber as String] as? CGWindowID,
              let boundsValue = window[kCGWindowBounds as String],
              CFGetTypeID(boundsValue as CFTypeRef) == CFDictionaryGetTypeID(),
              let bounds = CGRect(dictionaryRepresentation: boundsValue as! CFDictionary),
              bounds.width > 0, bounds.height > 0 else { return nil }
        return (number, bounds)
    }
    if let largest = candidates.max(by: { $0.1.width * $0.1.height < $1.1.width * $1.1.height }) {
        return largest
    }
    throw DriverError.message("no on-screen window found for pid \(pid)")
}

func windowStatus(pid: pid_t) -> [String: Any] {
    do {
        let (windowID, bounds) = try windowInfo(pid: pid)
        return [
            "ok": true,
            "visible": true,
            "window_id": windowID,
            "bounds": ["x": bounds.origin.x, "y": bounds.origin.y, "width": bounds.width, "height": bounds.height],
        ]
    } catch {
        return ["ok": true, "visible": false, "detail": String(describing: error)]
    }
}

func captureWindow(pid: pid_t, path: String) throws {
    let (windowID, bounds) = try windowInfo(pid: pid)
    guard let image = CGWindowListCreateImage(bounds, .optionIncludingWindow, windowID, [.boundsIgnoreFraming, .bestResolution]) else {
        throw DriverError.message("window screenshot failed")
    }
    let destination = URL(fileURLWithPath: path)
    try FileManager.default.createDirectory(at: destination.deletingLastPathComponent(), withIntermediateDirectories: true)
    guard let bitmap = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]) else {
        throw DriverError.message("PNG encoding failed")
    }
    try bitmap.write(to: destination, options: .atomic)
}

final class WindowRecorder: @unchecked Sendable {
    private var writer: AVAssetWriter?
    private var input: AVAssetWriterInput?
    private var adaptor: AVAssetWriterInputPixelBufferAdaptor?
    private var captureTask: Task<Void, Never>?
    private var captureError: Error?

    private func trace(_ message: String) {
        let timestamp = ISO8601DateFormatter().string(from: Date())
        FileHandle.standardError.write(Data("[\(timestamp)] record: \(message)\n".utf8))
    }

    func start(pid: pid_t, path: String) throws {
        let (windowID, bounds) = try windowInfo(pid: pid)
        let width = max(Int(bounds.width) & ~1, 2)
        let height = max(Int(bounds.height) & ~1, 2)
        let destination = URL(fileURLWithPath: path)
        try FileManager.default.createDirectory(at: destination.deletingLastPathComponent(), withIntermediateDirectories: true)
        try? FileManager.default.removeItem(at: destination)

        let assetWriter = try AVAssetWriter(outputURL: destination, fileType: .mp4)
        let settings: [String: Any] = [
            AVVideoCodecKey: AVVideoCodecType.h264,
            AVVideoWidthKey: width,
            AVVideoHeightKey: height,
        ]
        let writerInput = AVAssetWriterInput(mediaType: .video, outputSettings: settings)
        writerInput.expectsMediaDataInRealTime = true
        let attributes: [String: Any] = [
            kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
            kCVPixelBufferWidthKey as String: width,
            kCVPixelBufferHeightKey as String: height,
            kCVPixelBufferCGImageCompatibilityKey as String: true,
            kCVPixelBufferCGBitmapContextCompatibilityKey as String: true,
        ]
        let pixelAdaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: writerInput, sourcePixelBufferAttributes: attributes)
        guard assetWriter.canAdd(writerInput) else { throw DriverError.message("AVAssetWriter rejected window video input") }
        assetWriter.add(writerInput)
        guard assetWriter.startWriting() else { throw assetWriter.error ?? DriverError.message("AVAssetWriter failed to start") }
        assetWriter.startSession(atSourceTime: .zero)
        writer = assetWriter; input = writerInput; adaptor = pixelAdaptor
        trace("started CGWindow/AVAssetWriter backend window=\(windowID) size=\(width)x\(height)")

        captureTask = Task.detached { [weak self] in
            guard let self else { return }
            let start = ContinuousClock.now
            var cadence = CaptureCadence()
            while !Task.isCancelled {
                let target = start.advanced(by: .milliseconds(Int(cadence.frame * 1000 / 15)))
                try? await Task.sleep(until: target)
                if Task.isCancelled { break }
                autoreleasepool {
                    if !captureTick(
                        cadence: &cadence,
                        isReady: writerInput.isReadyForMoreMediaData,
                        append: { presentationTime in
                            guard let image = CGWindowListCreateImage(bounds, .optionIncludingWindow, windowID, [.boundsIgnoreFraming, .bestResolution]),
                                  let pool = pixelAdaptor.pixelBufferPool else { return true }
                            var optionalBuffer: CVPixelBuffer?
                            guard CVPixelBufferPoolCreatePixelBuffer(nil, pool, &optionalBuffer) == kCVReturnSuccess,
                                  let buffer = optionalBuffer else { return true }
                            CVPixelBufferLockBaseAddress(buffer, [])
                            defer { CVPixelBufferUnlockBaseAddress(buffer, []) }
                            guard let base = CVPixelBufferGetBaseAddress(buffer),
                                  let context = CGContext(data: base, width: width, height: height,
                                                          bitsPerComponent: 8, bytesPerRow: CVPixelBufferGetBytesPerRow(buffer),
                                                          space: CGColorSpaceCreateDeviceRGB(),
                                                          bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue) else { return true }
                            context.interpolationQuality = .high
                            context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
                            return pixelAdaptor.append(buffer, withPresentationTime: presentationTime)
                        }
                    ) {
                        self.captureError = assetWriter.error ?? DriverError.message("failed to append window video frame")
                    }
                }
            }
        }
    }

    func stop() async throws {
        captureTask?.cancel()
        _ = await captureTask?.result
        captureTask = nil
        input?.markAsFinished()
        if let assetWriter = writer { await assetWriter.finishWriting() }
        let error = captureError ?? writer?.error
        writer = nil; input = nil; adaptor = nil; captureError = nil
        if let error { throw error }
        trace("finalized recording")
    }
}

func jsonObject(_ data: Data) throws -> [String: Any] {
    guard let value = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
        throw DriverError.message("request must be a JSON object")
    }
    return value
}

func locator(from value: Any) throws -> Locator {
    let data = try JSONSerialization.data(withJSONObject: value)
    return try JSONDecoder().decode(Locator.self, from: data)
}

func response(_ object: [String: Any]) {
    let data = try! JSONSerialization.data(withJSONObject: object)
    print(String(data: data, encoding: .utf8)!)
    fflush(stdout)
}

func encoded<T: Encodable>(_ value: T) throws -> Any {
    let data = try JSONEncoder().encode(value)
    return try JSONSerialization.jsonObject(with: data)
}

func keyCode(_ key: String) throws -> CGKeyCode {
    switch key.lowercased() {
    case "tab": return 48
    case "return", "enter": return 36
    case "escape": return 53
    case "space": return 49
    default: throw DriverError.message("unsupported key: \(key)")
    }
}

@main
struct BuzzNativeDriver {
    static func main() async {
        do {
            let arguments = Array(CommandLine.arguments.dropFirst())
            guard let command = arguments.first else { throw DriverError.message("usage: buzz-native-driver doctor | serve --pid PID") }
            if command == "doctor" {
                let accessibility = AXIsProcessTrusted()
                let screen = CGPreflightScreenCaptureAccess()
                response(["ok": true, "checks": [
                    ["name": "native-driver-build", "ok": true, "detail": "Swift AX/CGEvent/Core Graphics/AVFoundation driver available"],
                    ["name": "accessibility-permission", "ok": accessibility, "detail": accessibility ? "granted" : "grant Accessibility to the invoking terminal/agent"],
                    ["name": "screen-recording-permission", "ok": screen, "detail": screen ? "granted" : "grant Screen Recording to the invoking terminal/agent"],
                    ["name": "recording-api", "ok": ProcessInfo.processInfo.isOperatingSystemAtLeast(.init(majorVersion: 15, minorVersion: 0, patchVersion: 0)), "detail": ProcessInfo.processInfo.operatingSystemVersionString]
                ]])
                return
            }
            guard command == "serve", (arguments.count == 3 || arguments.count == 5), arguments[1] == "--pid", let pid = pid_t(arguments[2]) else {
                throw DriverError.message("usage: buzz-native-driver serve --pid PID [--semantic-snapshot PATH]")
            }
            let semanticSnapshotPath: String? = arguments.count == 5 && arguments[3] == "--semantic-snapshot" ? arguments[4] : nil
            guard AXIsProcessTrusted() else { throw DriverError.message("Accessibility permission is not granted") }
            let app = AXUIElementCreateApplication(pid)
            enableWebViewAccessibility(app)
            var selected: (ElementDescription, Locator)?
            var recorder: Any?
            for try await line in FileHandle.standardInput.bytes.lines {
                do {
                    let request = try jsonObject(Data(line.utf8))
                    guard let name = request["command"] as? String else { throw DriverError.message("missing command") }
                    switch name {
                    case "locate":
                        guard let values = request["locators"] as? [Any] else { throw DriverError.message("locate requires locators") }
                        var found: (ElementDescription, Locator)?
                        for value in values {
                            let candidate = try locator(from: value)
                            if let path = semanticSnapshotPath,
                               let element = semanticNodes(path: path, pid: pid).first(where: { matches($0, locator: candidate) }) {
                                found = (describe(element, locator: candidate), candidate)
                                break
                            }
                            let semanticActive = semanticSnapshotPath.map { FileManager.default.fileExists(atPath: $0) } ?? false
                            if !semanticActive || candidate.role == "window" {
                                if let element = find(accessibilityRoots(app: app, pid: pid), locator: candidate) {
                                    found = (describe(element, locator: candidate), candidate)
                                    break
                                }
                            }
                        }
                        selected = found
                        if let (element, _) = found {
                            response(["ok": true, "element": try encoded(element)])
                        } else if (request["required"] as? Bool) == true {
                            throw DriverError.message("no accessibility element matched ordered locators")
                        } else { response(["ok": true, "element": NSNull()]) }
                    case "act":
                        guard let action = request["action"] as? [String: Any], let type = action["type"] as? String else { throw DriverError.message("act requires action.type") }
                        if type == "activate" {
                            guard let running = NSRunningApplication(processIdentifier: pid) else { throw DriverError.message("target app is no longer running") }
                            guard running.activate(options: [.activateIgnoringOtherApps]) else {
                                throw DriverError.message("target app refused activation")
                            }
                            let deadline = ContinuousClock.now.advanced(by: .seconds(2))
                            while !targetIsFrontmost(pid: pid), ContinuousClock.now < deadline {
                                try await Task.sleep(for: .milliseconds(25))
                            }
                            try requireTargetIsFrontmost(pid: pid)
                        } else if type == "wait" {
                            try await Task.sleep(for: .milliseconds(action["duration_ms"] as? Int ?? 0))
                        } else if type == "press" {
                            try requireTargetIsFrontmost(pid: pid)
                            let code = try keyCode(action["key"] as? String ?? "")
                            try postGlobalEvent(CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: true), pid: pid)
                            try postGlobalEvent(CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: false), pid: pid)
                        } else {
                            try requireTargetIsFrontmost(pid: pid)
                            guard let (_, used) = selected else {
                                throw DriverError.message("action requires a freshly selected element with current bounds")
                            }
                            var fresh: ElementDescription?
                            if let path = semanticSnapshotPath,
                               let element = semanticNodes(path: path, pid: pid).first(where: { matches($0, locator: used) }) {
                                fresh = describe(element, locator: used)
                            }
                            if fresh == nil, let element = find(accessibilityRoots(app: app, pid: pid), locator: used) {
                                fresh = describe(element, locator: used)
                            }
                            guard let element = fresh, let frame = element.frame else {
                                throw DriverError.message("action requires a freshly selected element with current bounds")
                            }
                            selected = (element, used)
                            let point = CGPoint(x: frame.x + frame.width / 2, y: frame.y + frame.height / 2)
                            let duration = max(action["duration_ms"] as? Int ?? 0, 0)
                            if type == "move_pointer" && duration > 0 {
                                let current = NSEvent.mouseLocation
                                let start = CGPoint(x: current.x, y: NSScreen.screens.first.map { $0.frame.height - current.y } ?? current.y)
                                let steps = max(duration / 8, 2)
                                let bounds = try windowInfo(pid: pid).1
                                for next in pointerPath(from: start, to: point, within: bounds, steps: steps) {
                                    try postGlobalPointerEvent(CGEvent(mouseEventSource: nil, mouseType: .mouseMoved, mouseCursorPosition: next, mouseButton: .left), at: next, pid: pid)
                                    try await Task.sleep(for: .milliseconds(duration / steps))
                                }
                            } else if type == "move_pointer" {
                                try postGlobalPointerEvent(CGEvent(mouseEventSource: nil, mouseType: .mouseMoved, mouseCursorPosition: point, mouseButton: .left), at: point, pid: pid)
                            } else if type == "click" {
                                try postGlobalPointerEvent(CGEvent(mouseEventSource: nil, mouseType: .mouseMoved, mouseCursorPosition: point, mouseButton: .left), at: point, pid: pid)
                                try postGlobalPointerEvent(CGEvent(mouseEventSource: nil, mouseType: .leftMouseDown, mouseCursorPosition: point, mouseButton: .left), at: point, pid: pid)
                                try postGlobalPointerEvent(CGEvent(mouseEventSource: nil, mouseType: .leftMouseUp, mouseCursorPosition: point, mouseButton: .left), at: point, pid: pid)
                            } else { throw DriverError.message("unsupported action: \(type)") }
                        }
                        response(["ok": true])
                    case "window_status":
                        response(windowStatus(pid: pid))
                    case "snapshot":
                        var budget = 20_000
                        var object: [String: Any] = ["ok": true, "tree": try encoded(snapshot(app, budget: &budget))]
                        if let path = semanticSnapshotPath {
                            object["semantic"] = try encoded(semanticNodes(path: path, pid: pid))
                        }
                        response(object)
                    case "screenshot":
                        guard let path = request["path"] as? String else { throw DriverError.message("screenshot requires path") }
                        try captureWindow(pid: pid, path: path); response(["ok": true])
                    case "focused":
                        response(["ok": true, "focused": selected?.0.focused ?? false])
                    case "selected":
                        if let (element, _) = selected { response(["ok": true, "element": try encoded(element)]) }
                        else { response(["ok": true, "element": NSNull()]) }
                    case "record_start":
                        guard let path = request["path"] as? String else { throw DriverError.message("record_start requires path") }
                        let value = WindowRecorder(); try value.start(pid: pid, path: path); recorder = value; response(["ok": true])
                    case "record_stop":
                        if let value = recorder as? WindowRecorder { try await value.stop() }
                        recorder = nil; response(["ok": true])
                    case "shutdown": response(["ok": true]); return
                    default: throw DriverError.message("unknown command: \(name)")
                    }
                } catch {
                    response(["ok": false, "error": String(describing: error)])
                }
            }
        } catch {
            response(["ok": false, "error": String(describing: error)])
            exit(1)
        }
    }
}
