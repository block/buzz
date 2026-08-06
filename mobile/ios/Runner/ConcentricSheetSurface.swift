import Flutter
import UIKit

final class ConcentricSheetSurfaceFactory: NSObject, FlutterPlatformViewFactory {
  func createArgsCodec() -> FlutterMessageCodec & NSObjectProtocol {
    FlutterStandardMessageCodec.sharedInstance()
  }

  func create(
    withFrame frame: CGRect,
    viewIdentifier viewId: Int64,
    arguments args: Any?
  ) -> FlutterPlatformView {
    ConcentricSheetSurfacePlatformView(frame: frame, arguments: args)
  }
}

final class ConcentricSheetSurfacePlatformView: NSObject, FlutterPlatformView {
  private let surfaceView: UIView

  init(frame: CGRect, arguments args: Any?) {
    let arguments = args as? [String: Any]
    let colorValue = (arguments?["color"] as? NSNumber)?.uint32Value ?? 0xFFFFFFFF
    let minimumRadius = (arguments?["minimumRadius"] as? NSNumber)?.doubleValue ?? 24

    surfaceView = UIView(frame: frame)
    surfaceView.isOpaque = true
    surfaceView.backgroundColor = Self.color(from: colorValue)
    surfaceView.clipsToBounds = true
    surfaceView.layer.cornerCurve = .continuous

    if #available(iOS 26.0, *) {
      surfaceView.cornerConfiguration = .uniformCorners(
        radius: .containerConcentric(minimum: minimumRadius)
      )
    } else {
      surfaceView.layer.cornerRadius = minimumRadius
    }

    super.init()
  }

  func view() -> UIView {
    surfaceView
  }

  private static func color(from value: UInt32) -> UIColor {
    let alpha = CGFloat((value >> 24) & 0xFF) / 255
    let red = CGFloat((value >> 16) & 0xFF) / 255
    let green = CGFloat((value >> 8) & 0xFF) / 255
    let blue = CGFloat(value & 0xFF) / 255
    return UIColor(red: red, green: green, blue: blue, alpha: alpha)
  }
}
