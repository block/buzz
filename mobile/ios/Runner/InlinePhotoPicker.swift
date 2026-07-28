import Flutter
import PhotosUI
import UIKit
import UniformTypeIdentifiers

final class InlinePhotoPickerFactory: NSObject, FlutterPlatformViewFactory {
  private let messenger: FlutterBinaryMessenger
  private weak var parentViewController: UIViewController?

  init(
    messenger: FlutterBinaryMessenger,
    parentViewController: UIViewController?
  ) {
    self.messenger = messenger
    self.parentViewController = parentViewController
    super.init()
  }

  func create(
    withFrame frame: CGRect,
    viewIdentifier viewId: Int64,
    arguments args: Any?
  ) -> FlutterPlatformView {
    InlinePhotoPickerPlatformView(
      frame: frame,
      viewIdentifier: viewId,
      messenger: messenger,
      parentViewController: parentViewController
    )
  }
}

final class InlinePhotoPickerPlatformView: NSObject, FlutterPlatformView {
  private let containerView: UIView
  private let channel: FlutterMethodChannel
  private weak var parentViewController: UIViewController?
  private var pickerViewController: PHPickerViewController?
  private var selectionGeneration = 0
  private var selectionTask: Task<Void, Never>?
  private var selectedTemporaryPaths: [String] = []

  init(
    frame: CGRect,
    viewIdentifier viewId: Int64,
    messenger: FlutterBinaryMessenger,
    parentViewController: UIViewController?
  ) {
    containerView = UIView(frame: frame)
    channel = FlutterMethodChannel(
      name: "buzz/inline_photo_picker/\(viewId)",
      binaryMessenger: messenger
    )
    self.parentViewController = parentViewController
    super.init()

    channel.setMethodCallHandler { [weak self] call, result in
      guard call.method == "claimSelection" else {
        result(FlutterMethodNotImplemented)
        return
      }
      let paths = call.arguments as? [String] ?? []
      guard let self, paths == self.selectedTemporaryPaths else {
        result(false)
        return
      }
      self.selectedTemporaryPaths = []
      result(true)
    }

    containerView.backgroundColor = .clear
    if #available(iOS 17.0, *) {
      installPicker()
    }
  }

  deinit {
    selectionTask?.cancel()
    Self.removeTemporaryFiles(selectedTemporaryPaths)
    channel.setMethodCallHandler(nil)
    pickerViewController?.willMove(toParent: nil)
    pickerViewController?.view.removeFromSuperview()
    pickerViewController?.removeFromParent()
  }

  func view() -> UIView {
    containerView
  }

  @available(iOS 17.0, *)
  private func installPicker() {
    var configuration = PHPickerConfiguration(photoLibrary: .shared())
    configuration.filter = .images
    configuration.selectionLimit = 0
    configuration.selection = .continuousAndOrdered
    configuration.preferredAssetRepresentationMode = .compatible
    configuration.disabledCapabilities = [
      .search,
      .stagingArea,
      .collectionNavigation,
      .selectionActions,
    ]
    configuration.edgesWithoutContentMargins = .all

    let picker = PHPickerViewController(configuration: configuration)
    picker.delegate = self
    picker.view.backgroundColor = .clear
    picker.view.translatesAutoresizingMaskIntoConstraints = false

    if let parentViewController {
      parentViewController.addChild(picker)
    }
    containerView.addSubview(picker.view)
    NSLayoutConstraint.activate([
      picker.view.leadingAnchor.constraint(equalTo: containerView.leadingAnchor),
      picker.view.trailingAnchor.constraint(equalTo: containerView.trailingAnchor),
      picker.view.topAnchor.constraint(equalTo: containerView.topAnchor),
      picker.view.bottomAnchor.constraint(equalTo: containerView.bottomAnchor),
    ])
    if parentViewController != nil {
      picker.didMove(toParent: parentViewController)
    }
    pickerViewController = picker
  }

  private func exportPickerResult(_ result: PHPickerResult) async throws -> String {
    let provider = result.itemProvider
    guard
      let typeIdentifier = provider.registeredTypeIdentifiers.first(where: {
        guard let type = UTType($0) else { return false }
        return type.conforms(to: .image)
      })
    else {
      throw InlinePhotoPickerError.unsupportedImage
    }

    return try await withCheckedThrowingContinuation { continuation in
      provider.loadFileRepresentation(forTypeIdentifier: typeIdentifier) {
        sourceURL,
        error in
        if let error {
          continuation.resume(throwing: error)
          return
        }
        guard let sourceURL else {
          continuation.resume(throwing: InlinePhotoPickerError.missingFile)
          return
        }

        do {
          let fileExtension =
            sourceURL.pathExtension.isEmpty
            ? (UTType(typeIdentifier)?.preferredFilenameExtension ?? "jpg")
            : sourceURL.pathExtension
          let destinationURL = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString)
            .appendingPathExtension(fileExtension)
          try FileManager.default.copyItem(
            at: sourceURL,
            to: destinationURL
          )
          continuation.resume(returning: destinationURL.path)
        } catch {
          continuation.resume(throwing: error)
        }
      }
    }
  }

  private static func removeTemporaryFiles(_ paths: [String]) {
    for path in paths where !path.isEmpty {
      try? FileManager.default.removeItem(atPath: path)
    }
  }
}

extension InlinePhotoPickerPlatformView: PHPickerViewControllerDelegate {
  func picker(
    _ picker: PHPickerViewController,
    didFinishPicking results: [PHPickerResult]
  ) {
    selectionGeneration += 1
    let generation = selectionGeneration
    selectionTask?.cancel()
    selectionTask = nil
    Self.removeTemporaryFiles(selectedTemporaryPaths)
    selectedTemporaryPaths = []
    channel.invokeMethod(
      "selectionCountChanged",
      arguments: results.count
    )

    guard !results.isEmpty else {
      channel.invokeMethod("selectionDidChange", arguments: [String]())
      return
    }

    selectionTask = Task { [weak self] in
      guard let self else { return }
      var paths: [String] = []
      do {
        for result in results {
          try Task.checkCancellation()
          paths.append(try await self.exportPickerResult(result))
        }
        try Task.checkCancellation()
        await MainActor.run {
          guard generation == self.selectionGeneration else {
            Self.removeTemporaryFiles(paths)
            return
          }
          self.selectedTemporaryPaths = paths
          self.selectionTask = nil
          self.channel.invokeMethod("selectionDidChange", arguments: paths)
        }
      } catch is CancellationError {
        Self.removeTemporaryFiles(paths)
      } catch {
        Self.removeTemporaryFiles(paths)
        await MainActor.run {
          guard generation == self.selectionGeneration else { return }
          self.selectionTask = nil
          self.channel.invokeMethod(
            "didFail",
            arguments: "Unable to prepare the selected photos."
          )
        }
      }
    }
  }
}

private enum InlinePhotoPickerError: Error {
  case missingFile
  case unsupportedImage
}
