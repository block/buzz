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
  private let corners: String
  private let usesGlass: Bool

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
    corners = arguments?["corners"] as? String ?? "all"
    usesGlass = (arguments?["usesGlass"] as? NSNumber)?.boolValue == true

    rootView = UIView(frame: frame)
    rootView.isUserInteractionEnabled = false
    rootView.isOpaque = !usesGlass && backdropColorValue != nil
    rootView.backgroundColor = backdropColorValue.map { Self.color(from: $0) } ?? .clear

    if #available(iOS 26.0, *), usesGlass {
      let effect = UIGlassEffect(style: .regular)
      effect.isInteractive = false
      surfaceView = UIVisualEffectView(effect: effect)
    } else {
      surfaceView = UIView()
    }
    surfaceView.frame = rootView.bounds
    surfaceView.isUserInteractionEnabled = false
    surfaceView.isOpaque = !usesGlass
    surfaceView.clipsToBounds = true
    surfaceView.layer.cornerCurve = .continuous
    surfaceView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
    rootView.addSubview(surfaceView)

    channel = FlutterMethodChannel(
      name: "buzz/concentric_sheet_surface/\(viewId)",
      binaryMessenger: messenger
    )

    super.init()

    updateColors(
      colorValue: colorValue,
      backdropColorValue: backdropColorValue
    )
    applyInterfaceStyle(from: arguments?["brightness"])
    applyCornerConfiguration(minimumRadius: minimumRadius)

    channel.setMethodCallHandler { [weak self] call, result in
      guard let self else {
        result(nil)
        return
      }
      switch call.method {
      case "updateColors":
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

        self.updateColors(
          colorValue: colorValue.uint32Value,
          backdropColorValue: (arguments["backdropColor"] as? NSNumber)?.uint32Value
        )
        result(nil)
      case "updateGeometry":
        guard
          let arguments = call.arguments as? [String: Any],
          let minimumRadius = arguments["minimumRadius"] as? NSNumber
        else {
          result(
            FlutterError(
              code: "invalid_arguments",
              message: "Expected a minimum corner radius.",
              details: nil
            )
          )
          return
        }
        self.applyInterfaceStyle(from: arguments["brightness"])
        self.applyCornerConfiguration(
          minimumRadius: minimumRadius.doubleValue
        )
        result(nil)
      default:
        result(FlutterMethodNotImplemented)
      }
    }
  }

  deinit {
    channel.setMethodCallHandler(nil)
  }

  func view() -> UIView {
    rootView
  }

  private func updateColors(colorValue: UInt32, backdropColorValue: UInt32?) {
    let surfaceColor = Self.color(from: colorValue)
    if let glassView = surfaceView as? UIVisualEffectView {
      glassView.backgroundColor = .clear
      glassView.contentView.backgroundColor = surfaceColor.withAlphaComponent(0.12)
    } else {
      surfaceView.backgroundColor = surfaceColor
    }
    rootView.isOpaque = !usesGlass && backdropColorValue != nil
    rootView.backgroundColor = backdropColorValue.map { Self.color(from: $0) } ?? .clear
  }

  private func applyCornerConfiguration(minimumRadius: CGFloat) {
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
  }

  private func applyInterfaceStyle(from value: Any?) {
    switch value as? String {
    case "dark":
      rootView.overrideUserInterfaceStyle = .dark
    case "light":
      rootView.overrideUserInterfaceStyle = .light
    default:
      rootView.overrideUserInterfaceStyle = .unspecified
    }
  }

  private static func color(from value: UInt32) -> UIColor {
    let alpha = CGFloat((value >> 24) & 0xFF) / 255
    let red = CGFloat((value >> 16) & 0xFF) / 255
    let green = CGFloat((value >> 8) & 0xFF) / 255
    let blue = CGFloat(value & 0xFF) / 255
    return UIColor(red: red, green: green, blue: blue, alpha: alpha)
  }
}
