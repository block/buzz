import Flutter
import UIKit

final class ConcentricSheetSurfaceFactory: NSObject, FlutterPlatformViewFactory {
  private let messenger: FlutterBinaryMessenger

  init(messenger: FlutterBinaryMessenger) {
    self.messenger = messenger
    super.init()
  }

  func createArgsCodec() -> FlutterMessageCodec & NSObjectProtocol {
    FlutterStandardMessageCodec.sharedInstance()
  }

  func create(
    withFrame frame: CGRect,
    viewIdentifier viewId: Int64,
    arguments args: Any?
  ) -> FlutterPlatformView {
    ConcentricSheetSurfacePlatformView(
      frame: frame,
      viewIdentifier: viewId,
      messenger: messenger,
      arguments: args
    )
  }
}

final class ConcentricSheetSurfacePlatformView: NSObject, FlutterPlatformView {
  private let rootView: UIView
  private let surfaceView: UIView
  private let channel: FlutterMethodChannel

  init(
    frame: CGRect,
    viewIdentifier viewId: Int64,
    messenger: FlutterBinaryMessenger,
    arguments args: Any?
  ) {
    let arguments = args as? [String: Any]
    let colorValue = (arguments?["color"] as? NSNumber)?.uint32Value ?? 0xFFFF_FFFF
    let backdropColorValue = (arguments?["backdropColor"] as? NSNumber)?.uint32Value
    let minimumRadius = (arguments?["minimumRadius"] as? NSNumber)?.doubleValue ?? 24
    let corners = arguments?["corners"] as? String ?? "all"

    rootView = UIView(frame: frame)
    rootView.isOpaque = backdropColorValue != nil
    rootView.backgroundColor = backdropColorValue.map { Self.color(from: $0) } ?? .clear

    surfaceView = UIView(frame: rootView.bounds)
    surfaceView.isOpaque = true
    surfaceView.backgroundColor = Self.color(from: colorValue)
    surfaceView.clipsToBounds = true
    surfaceView.layer.cornerCurve = .continuous
    surfaceView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
    rootView.addSubview(surfaceView)

    channel = FlutterMethodChannel(
      name: "buzz/concentric_sheet_surface/\(viewId)",
      binaryMessenger: messenger
    )

    if #available(iOS 26.0, *) {
      let radius = UICornerRadius.containerConcentric(minimum: minimumRadius)
      surfaceView.cornerConfiguration =
        corners == "bottom"
        ? .uniformBottomRadius(
          radius,
          topLeftRadius: nil,
          topRightRadius: nil
        )
        : .uniformCorners(radius: radius)
    } else {
      surfaceView.layer.cornerRadius = minimumRadius
      if corners == "bottom" {
        surfaceView.layer.maskedCorners = [
          .layerMinXMaxYCorner,
          .layerMaxXMaxYCorner,
        ]
      }
    }

    super.init()

    channel.setMethodCallHandler { [weak self] call, result in
      guard call.method == "updateColors" else {
        result(FlutterMethodNotImplemented)
        return
      }
      guard
        let arguments = call.arguments as? [String: Any],
        let colorValue = arguments["color"] as? NSNumber
      else {
        result(
          FlutterError(
            code: "invalid_arguments",
            message: "Expected a surface color.",
            details: nil
          )
        )
        return
      }

      self?.updateColors(
        colorValue: colorValue.uint32Value,
        backdropColorValue: (arguments["backdropColor"] as? NSNumber)?.uint32Value
      )
      result(nil)
    }
  }

  deinit {
    channel.setMethodCallHandler(nil)
  }

  func view() -> UIView {
    rootView
  }

  private func updateColors(colorValue: UInt32, backdropColorValue: UInt32?) {
    surfaceView.backgroundColor = Self.color(from: colorValue)
    rootView.isOpaque = backdropColorValue != nil
    rootView.backgroundColor = backdropColorValue.map { Self.color(from: $0) } ?? .clear
  }

  private static func color(from value: UInt32) -> UIColor {
    let alpha = CGFloat((value >> 24) & 0xFF) / 255
    let red = CGFloat((value >> 16) & 0xFF) / 255
    let green = CGFloat((value >> 8) & 0xFF) / 255
    let blue = CGFloat(value & 0xFF) / 255
    return UIColor(red: red, green: green, blue: blue, alpha: alpha)
  }
}
