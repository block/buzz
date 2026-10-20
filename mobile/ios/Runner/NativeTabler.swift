import UIKit
import CoreText
import Flutter

/// Renders the same bundled Tabler font used by Flutter into template images.
/// Legacy symbol keys are kept at the platform-channel boundary only.
enum NativeTabler {
  private static let fontName: String? = {
    let key = FlutterDartProject.lookupKey(forAsset: "assets/fonts/TablerIcons.ttf")
    let appBundle = Bundle(url: Bundle.main.bundleURL.appendingPathComponent("Frameworks/App.framework"))
    guard let url = appBundle?.url(forResource: key, withExtension: nil) ?? Bundle.main.url(forResource: key, withExtension: nil),
      let provider = CGDataProvider(url: url as CFURL),
      let font = CGFont(provider) else { return nil }
    CTFontManagerRegisterGraphicsFont(font, nil)
    return font.postScriptName as String?
  }()

  static func image(_ symbol: String, size: CGFloat = 20) -> UIImage? {
    guard let code = glyphs[symbol], let scalar = UnicodeScalar(code),
      let name = fontName, let font = UIFont(name: name, size: size) else {
      assertionFailure("Missing Tabler icon: \(symbol)")
      return nil
    }
    let text = String(scalar) as NSString
    let attributes: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: UIColor.black]
    let measured = text.size(withAttributes: attributes)
    return UIGraphicsImageRenderer(size: CGSize(width: size, height: size)).image { _ in
      text.draw(at: CGPoint(x: (size - measured.width) / 2, y: (size - measured.height) / 2), withAttributes: attributes)
    }.withRenderingMode(.alwaysTemplate)
  }

  private static let glyphs: [String: UInt32] = [
    "arrow.down": 0xea16, // arrow-down
    "xmark": 0xeb55, // x
    "xmark.circle.fill": 0xea6a, // circle-x
    "chevron.backward": 0xea60, // chevron-left
    "chevron.left": 0xea60, // chevron-left
    "chevron.right": 0xea61, // chevron-right
    "camera": 0xea54, // camera
    "photo.on.rectangle": 0xeb0a, // photo
    "photo.on.rectangle.angled": 0xeb0a, // photo
    "paintpalette": 0xeb01, // palette
    "drop": 0xea97, // droplet
    "drop.fill": 0xea97, // droplet
    "face.smiling": 0xeaf7, // mood-smile
    "person": 0xeb4d, // user
    "person.crop.circle": 0xeb4d, // user
    "person.2": 0xebf2, // users
    "rectangle.stack": 0xeb0a, // photo
    "crop": 0xea85, // crop
    "arrow.triangle.2.circlepath.camera": 0xee22, // camera-rotate
    "circle": 0xea6b, // circle
    "circle.fill": 0xea6b, // circle
    "sun.max": 0xeb30, // sun
    "moon": 0xeaf8, // moon
    "ellipsis": 0xea95, // dots
    "arrowshape.turn.up.left": 0xeb77, // arrow-back-up
    "circle.lefthalf.filled": 0xf4a3, // sun-moon
    "magnifyingglass": 0xeb1c, // search
    "tray": 0xeac4, // inbox
    "sparkles": 0xf6d7, // sparkles
    "clock": 0xea70, // clock
    "leaf": 0xed4f, // leaf
    "fork.knife": 0xeeff, // tools-kitchen-2
    "figure.run": 0xec82, // run
    "airplane": 0xeb6f, // plane
    "lightbulb": 0xea51, // bulb
    "heart": 0xeabe, // heart
    "flag": 0xeaa6, // flag
    "video": 0xed22, // video
    "mic": 0xeaf0, // microphone
    "doc": 0xeaa4, // file
    "plus": 0xeb0b, // plus
    "building.2.crop.circle": 0xebf6, // building-community
    "folder.badge.plus": 0xeaab, // folder-plus
    "headphones": 0xeabd, // headphones
    "line.3.horizontal.decrease": 0xeaa5, // filter
    "square.and.pencil": 0xea98, // edit
    "pencil": 0xea98, // edit
    "doc.on.doc": 0xea7a, // copy
    "trash": 0xeb41, // trash
    "pin": 0xec9c, // pin
    "pin.slash": 0xed5f, // pinned-off
    "square.and.arrow.up": 0xeb21, // share
    "link": 0xeade, // link
    "checkmark": 0xea5e, // check
    "checkmark.circle": 0xea67, // circle-check
    "bell.slash": 0xece9, // bell-off
    "bubble.left": 0xeaef, // message
    "bubble.left.and.bubble.right": 0xeb6c, // messages
    "arrow.uturn.backward": 0xeb77, // arrow-back-up
  ]
}
