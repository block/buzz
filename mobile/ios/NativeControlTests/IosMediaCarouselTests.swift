import Flutter
import UIKit

@main
struct IosMediaCarouselTests {
  @MainActor static func main() {
    let messenger = CarouselTestMessenger()
    var args: [String: Any] = [
      "labels": ["Photo A", "Photo B", "Photo C"],
      "width": 300.0, "gutter": 20.0, "gap": 4.0, "trailingPadding": 20.0, "radius": 16.0,
    ]
    let platform = IosMediaCarousel(
      frame: CGRect(x: 0, y: 0, width: 340, height: 240), id: 42,
      arguments: args, messenger: messenger
    )
    guard let strip = platform.view() as? UICollectionView else {
      preconditionFailure("Carousel must use native collection scrolling")
    }
    messenger.configure(args)
    strip.layoutIfNeeded()
    precondition(strip.numberOfItems(inSection: 0) == 3)
    precondition(strip.contentOffset.x == -20, "Initial photo starts at gutter")
    precondition(strip.decelerationRate == .fast)
    let first = IndexPath(item: 0, section: 0)
    guard let cell = strip.cellForItem(at: first) else {
      preconditionFailure("Missing photo cell")
    }
    precondition(cell.accessibilityLabel == "Open Photo A")
    precondition(cell.accessibilityTraits.contains(.button))
    precondition(messenger.calls.contains { $0.method == "image" }, "Images requested after ready")
    platform.collectionView(strip, didSelectItemAt: IndexPath(item: 1, section: 0))
    precondition(messenger.calls.last?.method == "open")
    precondition(messenger.calls.last?.arguments as? Int == 1)
    var target = CGPoint(x: 235, y: 0)
    platform.scrollViewWillEndDragging(strip, withVelocity: CGPoint(x: 1, y: 0), targetContentOffset: &target)
    precondition(target.x == 250, "Drag snaps to native item stride")
    target.x = 10000
    platform.scrollViewWillEndDragging(strip, withVelocity: .zero, targetContentOffset: &target)
    precondition(target.x <= strip.contentSize.width - strip.bounds.width + 20, "No trailing blank page")
    let last = IndexPath(item: 2, section: 0)
    guard let lastFrame = strip.layoutAttributesForItem(at: last)?.frame else {
      preconditionFailure("Missing final photo layout")
    }
    let trailingEdge = lastFrame.maxX - target.x
    precondition(abs((strip.bounds.width - 20) - trailingEdge - 20) < 0.01,
      "Final photo must stop at the full 20pt message gutter, not the 4pt photo gap")
    precondition(lastFrame.minX == 540, "Trailing padding must not change page stride")
    strip.contentOffset.x = 250
    platform.scrollViewDidScroll(strip)
    precondition(messenger.calls.contains { $0.method == "page" && $0.arguments as? Int == 1 })
    args["rtl"] = true
    messenger.configure(args)
    precondition(strip.transform.a == -1, "Native strip follows RTL reading order")
    strip.layoutIfNeeded()
    let rtlCell = strip.visibleCells.first
    precondition(rtlCell?.transform.a == -1, "Photo itself is not mirrored")
    target.x = 10000
    platform.scrollViewWillEndDragging(strip, withVelocity: .zero, targetContentOffset: &target)
    guard let rtlLast = strip.layoutAttributesForItem(at: last)?.frame else {
      preconditionFailure("Missing RTL final photo layout")
    }
    precondition(abs((strip.bounds.width - 20) - (rtlLast.maxX - target.x) - 20) < 0.01,
      "RTL final photo must retain the same message gutter")
    args["width"] = 200.0
    messenger.configure(args)
    precondition(strip.contentOffset.x == 160, "Resize retains the selected logical photo")
    print("Native carousel scrolling, bounds, RTL, image requests, and taps passed")
  }
}

private final class CarouselTestMessenger: NSObject, FlutterBinaryMessenger {
  var calls: [FlutterMethodCall] = []
  var handler: FlutterBinaryMessageHandler?
  func configure(_ args: [String: Any]) {
    let codec = FlutterStandardMethodCodec.sharedInstance()
    handler?(codec.encode(FlutterMethodCall(methodName: "configure", arguments: args)), { _ in })
  }
  func send(onChannel channel: String, message: Data?) {
    guard let message else { return }
    calls.append(FlutterStandardMethodCodec.sharedInstance().decodeMethodCall(message))
  }
  func send(onChannel channel: String, message: Data?, binaryReply callback: FlutterBinaryReply?) {
    send(onChannel: channel, message: message)
    callback?(FlutterStandardMethodCodec.sharedInstance().encodeSuccessEnvelope(nil))
  }
  func setMessageHandlerOnChannel(_ channel: String, binaryMessageHandler handler: FlutterBinaryMessageHandler?) -> FlutterBinaryMessengerConnection {
    self.handler = handler
    return 1
  }
  func cleanUpConnection(_ connection: FlutterBinaryMessengerConnection) {}
}
