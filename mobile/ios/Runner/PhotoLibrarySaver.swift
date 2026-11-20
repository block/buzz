import Flutter
import Photos

/// Adds an image to Photos with add-only access.
///
/// PhotoKit lets an add-only app create an asset but never fetch one, so this
/// reports the result of the write itself and never reads the new asset back.
final class PhotoLibrarySaver {
  private let channel: FlutterMethodChannel

  init(messenger: FlutterBinaryMessenger) {
    channel = FlutterMethodChannel(name: "buzz/photo_library_saver", binaryMessenger: messenger)
    channel.setMethodCallHandler { call, result in
      guard call.method == "saveImage" else {
        result(FlutterMethodNotImplemented)
        return
      }
      guard let arguments = call.arguments as? [String: Any],
        let bytes = arguments["bytes"] as? FlutterStandardTypedData,
        let filename = arguments["filename"] as? String
      else {
        result(FlutterError(code: "invalid_arguments", message: nil, details: nil))
        return
      }
      PHPhotoLibrary.shared().performChanges {
        let options = PHAssetResourceCreationOptions()
        options.originalFilename = filename
        PHAssetCreationRequest.forAsset().addResource(
          with: .photo, data: bytes.data, options: options)
      } completionHandler: { saved, error in
        DispatchQueue.main.async {
          if saved {
            result(nil)
          } else {
            result(
              FlutterError(
                code: "save_failed", message: error?.localizedDescription, details: nil))
          }
        }
      }
    }
  }
}
