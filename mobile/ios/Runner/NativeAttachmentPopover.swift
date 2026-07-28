import AVFoundation
import Flutter
import PhotosUI
import UIKit
import UniformTypeIdentifiers

final class NativeAttachmentPopoverCoordinator: NSObject {
  private let channel: FlutterMethodChannel
  private weak var parentViewController: UIViewController?
  private weak var presentedController: UIViewController?
  private weak var sourceAnchorView: UIView?

  init(
    messenger: FlutterBinaryMessenger,
    parentViewController: UIViewController?
  ) {
    channel = FlutterMethodChannel(
      name: "buzz/native_attachment_popover",
      binaryMessenger: messenger
    )
    self.parentViewController = parentViewController
    super.init()

    channel.setMethodCallHandler { [weak self] call, result in
      self?.handle(call, result: result)
    }
  }

  private func handle(
    _ call: FlutterMethodCall,
    result: @escaping FlutterResult
  ) {
    switch call.method {
    case "isSupported":
      if #available(iOS 26.0, *) {
        result(true)
      } else {
        result(false)
      }
    case "present":
      guard
        let arguments = call.arguments as? [String: Any],
        let x = arguments["x"] as? NSNumber,
        let y = arguments["y"] as? NSNumber,
        let width = arguments["width"] as? NSNumber,
        let height = arguments["height"] as? NSNumber
      else {
        result(
          FlutterError(
            code: "invalid_arguments",
            message: "Expected the attachment trigger bounds.",
            details: nil
          )
        )
        return
      }

      let sourceRect = CGRect(
        x: CGFloat(truncating: x),
        y: CGFloat(truncating: y),
        width: CGFloat(truncating: width),
        height: CGFloat(truncating: height)
      )
      DispatchQueue.main.async { [weak self] in
        result(self?.presentPopover(sourceRect: sourceRect) ?? false)
      }
    case "dismiss":
      DispatchQueue.main.async { [weak self] in
        if #available(iOS 26.0, *),
          let controller =
            self?.presentedController
            as? NativeAttachmentPopoverViewController
        {
          controller.dismissAndNotify()
        } else {
          self?.presentedController?.dismiss(animated: true)
        }
        result(nil)
      }
    default:
      result(FlutterMethodNotImplemented)
    }
  }

  @MainActor
  private func presentPopover(sourceRect: CGRect) -> Bool {
    guard #available(iOS 26.0, *) else { return false }
    guard presentedController == nil else { return true }
    let rootViewController =
      parentViewController ?? activeWindowRootViewController()
    guard let presenter = topViewController(from: rootViewController) else {
      return false
    }

    let sourceView = presenter.view
    let convertedRect: CGRect
    if let window = sourceView?.window {
      convertedRect = sourceView?.convert(sourceRect, from: window) ?? sourceRect
    } else {
      convertedRect = sourceRect
    }

    let anchorView = makeSourceAnchor(frame: convertedRect)
    sourceView?.addSubview(anchorView)
    sourceAnchorView = anchorView

    let availableWidth = max(
      320,
      min(
        (sourceView?.bounds.width ?? UIScreen.main.bounds.width) - 24,
        430
      )
    )
    let controller = NativeAttachmentPopoverViewController(
      channel: channel,
      expandedWidth: availableWidth
    )
    controller.modalPresentationStyle = .popover
    controller.preferredTransition = .zoom { [weak anchorView] _ in
      anchorView
    }
    controller.onDismiss = { [weak self] in
      self?.presentedController = nil
      self?.sourceAnchorView?.removeFromSuperview()
      self?.channel.invokeMethod("dismissed", arguments: nil)
    }

    guard let popover = controller.popoverPresentationController else {
      anchorView.removeFromSuperview()
      return false
    }
    popover.sourceView = anchorView
    popover.sourceRect = anchorView.bounds
    popover.permittedArrowDirections = [.down]
    popover.backgroundColor = .clear
    popover.delegate = controller

    presentedController = controller
    presenter.present(controller, animated: true)
    return true
  }

  @MainActor
  private func makeSourceAnchor(frame: CGRect) -> UIView {
    let anchor = UIView(frame: frame)
    anchor.isUserInteractionEnabled = false
    anchor.accessibilityElementsHidden = true
    anchor.backgroundColor = .clear
    anchor.layer.cornerRadius = min(frame.width, frame.height) / 2
    anchor.layer.cornerCurve = .continuous
    return anchor
  }

  @MainActor
  private func activeWindowRootViewController() -> UIViewController? {
    UIApplication.shared.connectedScenes
      .compactMap { $0 as? UIWindowScene }
      .filter { $0.activationState == .foregroundActive }
      .flatMap(\.windows)
      .first(where: \.isKeyWindow)?
      .rootViewController
  }

  @MainActor
  private func topViewController(
    from viewController: UIViewController?
  ) -> UIViewController? {
    if let presented = viewController?.presentedViewController {
      return topViewController(from: presented)
    }
    if let navigation = viewController as? UINavigationController {
      return topViewController(from: navigation.visibleViewController)
    }
    if let tab = viewController as? UITabBarController {
      return topViewController(from: tab.selectedViewController)
    }
    return viewController
  }
}

@available(iOS 26.0, *)
private final class NativeAttachmentPopoverViewController:
  UIViewController,
  PHPickerViewControllerDelegate,
  UIPopoverPresentationControllerDelegate,
  AVCapturePhotoCaptureDelegate
{
  private enum Surface {
    case menu
    case photos
    case camera
  }

  private let channel: FlutterMethodChannel
  private let expandedWidth: CGFloat
  private let menuSize = CGSize(width: 176, height: 208)
  private let expandedHeight: CGFloat = 372
  private let expandedHorizontalOffset: CGFloat = 10
  private let expandedVerticalOffset: CGFloat = 40
  private let contentHost = UIView()
  private let cameraSession = AVCaptureSession()
  private let cameraOutput = AVCapturePhotoOutput()
  private let cameraQueue = DispatchQueue(
    label: "buzz.native-attachment-camera"
  )

  private var surface = Surface.menu
  private var visibleContentView: UIView?
  private var photoPickerViewController: PHPickerViewController?
  private var cameraPreviewLayer: AVCaptureVideoPreviewLayer?
  private weak var cameraPreviewView: UIView?
  private weak var photoActionButton: UIButton?
  private weak var cameraCaptureButton: UIButton?
  private var selectionGeneration = 0
  private var selectedPhotoPaths: [String] = []
  private var cameraConfigured = false
  private var cameraIsStarting = false
  private var cameraIsCapturing = false
  private var didNotifyDismissal = false

  var onDismiss: (() -> Void)?

  init(channel: FlutterMethodChannel, expandedWidth: CGFloat) {
    self.channel = channel
    self.expandedWidth = expandedWidth
    super.init(nibName: nil, bundle: nil)
    preferredContentSize = menuSize
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) {
    fatalError("init(coder:) has not been implemented")
  }

  override func viewDidLoad() {
    super.viewDidLoad()
    view.backgroundColor = .clear
    view.layer.cornerRadius = 22
    view.layer.cornerCurve = .continuous
    view.clipsToBounds = true

    let glassEffect = UIGlassEffect(style: .regular)
    glassEffect.isInteractive = true
    let glassView = UIVisualEffectView(effect: glassEffect)
    glassView.translatesAutoresizingMaskIntoConstraints = false
    view.addSubview(glassView)

    contentHost.translatesAutoresizingMaskIntoConstraints = false
    view.addSubview(contentHost)
    NSLayoutConstraint.activate([
      glassView.leadingAnchor.constraint(equalTo: view.leadingAnchor),
      glassView.trailingAnchor.constraint(equalTo: view.trailingAnchor),
      glassView.topAnchor.constraint(equalTo: view.topAnchor),
      glassView.bottomAnchor.constraint(equalTo: view.bottomAnchor),
      contentHost.leadingAnchor.constraint(equalTo: view.leadingAnchor),
      contentHost.trailingAnchor.constraint(equalTo: view.trailingAnchor),
      contentHost.topAnchor.constraint(equalTo: view.topAnchor),
      contentHost.bottomAnchor.constraint(equalTo: view.bottomAnchor),
    ])

    let menu = makeMenuView()
    installContent(menu)
  }

  override func viewDidLayoutSubviews() {
    super.viewDidLayoutSubviews()
    cameraPreviewLayer?.frame = cameraPreviewView?.bounds ?? .zero
  }

  override func viewWillDisappear(_ animated: Bool) {
    super.viewWillDisappear(animated)
    stopCamera()
  }

  func adaptivePresentationStyle(
    for controller: UIPresentationController
  ) -> UIModalPresentationStyle {
    .none
  }

  func presentationControllerDidDismiss(
    _ presentationController: UIPresentationController
  ) {
    notifyDismissalIfNeeded()
  }

  private func makeMenuView() -> UIView {
    let container = UIView()
    container.translatesAutoresizingMaskIntoConstraints = false

    let stack = UIStackView()
    stack.axis = .vertical
    stack.distribution = .fillEqually
    stack.translatesAutoresizingMaskIntoConstraints = false
    container.addSubview(stack)
    NSLayoutConstraint.activate([
      stack.leadingAnchor.constraint(equalTo: container.leadingAnchor),
      stack.trailingAnchor.constraint(equalTo: container.trailingAnchor),
      stack.topAnchor.constraint(equalTo: container.topAnchor, constant: 8),
      stack.bottomAnchor.constraint(equalTo: container.bottomAnchor, constant: -8),
    ])

    stack.addArrangedSubview(
      makeMenuButton(
        title: "Camera",
        symbol: "camera",
        action: UIAction { [weak self] _ in self?.showCamera() }
      )
    )
    stack.addArrangedSubview(
      makeMenuButton(
        title: "Photos",
        symbol: "photo.on.rectangle.angled",
        action: UIAction { [weak self] _ in self?.showPhotos() }
      )
    )
    stack.addArrangedSubview(
      makeMenuButton(
        title: "Video",
        symbol: "video",
        action: UIAction { [weak self] _ in
          self?.finish(method: "pickVideo")
        }
      )
    )
    stack.addArrangedSubview(
      makeMenuButton(
        title: "Files",
        symbol: "doc",
        action: UIAction { [weak self] _ in
          self?.finish(method: "pickFiles")
        }
      )
    )
    return container
  }

  private func makeMenuButton(
    title: String,
    symbol: String,
    action: UIAction
  ) -> UIButton {
    let button = UIButton(primaryAction: action)
    button.accessibilityLabel = title

    let symbolConfiguration = UIImage.SymbolConfiguration(
      pointSize: 18,
      weight: .regular
    )
    let iconView = UIImageView(
      image: UIImage(
        systemName: symbol,
        withConfiguration: symbolConfiguration
      )
    )
    iconView.tintColor = .label
    iconView.contentMode = .center
    iconView.translatesAutoresizingMaskIntoConstraints = false

    let titleLabel = UILabel()
    titleLabel.text = title
    titleLabel.textColor = .label
    titleLabel.font = .preferredFont(forTextStyle: .body)
    titleLabel.adjustsFontForContentSizeCategory = true
    titleLabel.textAlignment = .left
    titleLabel.translatesAutoresizingMaskIntoConstraints = false

    button.addSubview(iconView)
    button.addSubview(titleLabel)
    NSLayoutConstraint.activate([
      iconView.leadingAnchor.constraint(equalTo: button.leadingAnchor, constant: 14),
      iconView.centerYAnchor.constraint(equalTo: button.centerYAnchor),
      iconView.widthAnchor.constraint(equalToConstant: 26),
      titleLabel.leadingAnchor.constraint(
        equalTo: iconView.trailingAnchor,
        constant: 12
      ),
      titleLabel.trailingAnchor.constraint(
        equalTo: button.trailingAnchor,
        constant: -14
      ),
      titleLabel.centerYAnchor.constraint(equalTo: button.centerYAnchor),
    ])
    button.configurationUpdateHandler = { button in
      button.alpha = button.isHighlighted ? 0.62 : 1
    }
    return button
  }

  private func showPhotos() {
    guard surface != .photos else { return }
    stopCamera()

    var configuration = PHPickerConfiguration(photoLibrary: .shared())
    configuration.filter = .images
    configuration.selectionLimit = 0
    configuration.selection = .continuousAndOrdered
    configuration.preferredAssetRepresentationMode = .current
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

    let container = UIView()
    container.backgroundColor = .clear
    addChild(picker)
    picker.view.translatesAutoresizingMaskIntoConstraints = false
    container.addSubview(picker.view)
    NSLayoutConstraint.activate([
      picker.view.leadingAnchor.constraint(equalTo: container.leadingAnchor),
      picker.view.trailingAnchor.constraint(equalTo: container.trailingAnchor),
      picker.view.topAnchor.constraint(
        equalTo: container.topAnchor,
        constant: -8
      ),
      picker.view.bottomAnchor.constraint(equalTo: container.bottomAnchor),
    ])
    picker.didMove(toParent: self)
    photoPickerViewController = picker

    let backButton = makeGlassControl(
      title: nil,
      symbol: "chevron.left",
      accessibilityLabel: "Back to attachment options",
      action: UIAction { [weak self] _ in self?.showMenu() }
    )
    let actionButton = makeGlassControl(
      title: "All Photos",
      symbol: nil,
      accessibilityLabel: "All Photos",
      prominent: true,
      action: UIAction { [weak self] _ in self?.performPhotoAction() }
    )
    photoActionButton = actionButton
    addBottomControls(
      to: container,
      leading: backButton,
      trailing: actionButton
    )

    transition(to: .photos, content: container)
  }

  private func showCamera() {
    guard surface != .camera else { return }
    removePhotoPicker()

    let container = UIView()
    container.backgroundColor = .black
    let preview = UIView()
    preview.backgroundColor = .black
    preview.translatesAutoresizingMaskIntoConstraints = false
    container.addSubview(preview)
    NSLayoutConstraint.activate([
      preview.leadingAnchor.constraint(equalTo: container.leadingAnchor),
      preview.trailingAnchor.constraint(equalTo: container.trailingAnchor),
      preview.topAnchor.constraint(equalTo: container.topAnchor),
      preview.bottomAnchor.constraint(equalTo: container.bottomAnchor),
    ])
    cameraPreviewView = preview

    let placeholder = UIActivityIndicatorView(style: .large)
    placeholder.color = .white
    placeholder.startAnimating()
    placeholder.translatesAutoresizingMaskIntoConstraints = false
    preview.addSubview(placeholder)
    NSLayoutConstraint.activate([
      placeholder.centerXAnchor.constraint(equalTo: preview.centerXAnchor),
      placeholder.centerYAnchor.constraint(equalTo: preview.centerYAnchor),
    ])
    placeholder.tag = 7001

    let backButton = makeGlassControl(
      title: nil,
      symbol: "chevron.left",
      accessibilityLabel: "Back to attachment options",
      action: UIAction { [weak self] _ in self?.showMenu() }
    )
    let captureButton = makeCameraCaptureButton()
    cameraCaptureButton = captureButton
    addBottomControls(
      to: container,
      leading: backButton,
      center: captureButton
    )

    transition(to: .camera, content: container)
    startCamera()
  }

  private func showMenu() {
    guard surface != .menu else { return }
    selectionGeneration += 1
    selectedPhotoPaths = []
    stopCamera()

    let menu = makeMenuView()
    transition(to: .menu, content: menu) { [weak self] in
      self?.removePhotoPicker()
    }
  }

  private func installContent(_ content: UIView) {
    content.translatesAutoresizingMaskIntoConstraints = false
    contentHost.addSubview(content)
    NSLayoutConstraint.activate([
      content.leadingAnchor.constraint(equalTo: contentHost.leadingAnchor),
      content.trailingAnchor.constraint(equalTo: contentHost.trailingAnchor),
      content.topAnchor.constraint(equalTo: contentHost.topAnchor),
      content.bottomAnchor.constraint(equalTo: contentHost.bottomAnchor),
    ])
    visibleContentView = content
  }

  private func transition(
    to nextSurface: Surface,
    content nextView: UIView,
    completion: (() -> Void)? = nil
  ) {
    let previousView = visibleContentView
    let isExpanding = nextSurface != .menu
    let targetSize =
      isExpanding
      ? CGSize(width: expandedWidth, height: expandedHeight)
      : menuSize

    nextView.translatesAutoresizingMaskIntoConstraints = false
    contentHost.addSubview(nextView)
    NSLayoutConstraint.activate([
      nextView.leadingAnchor.constraint(equalTo: contentHost.leadingAnchor),
      nextView.trailingAnchor.constraint(equalTo: contentHost.trailingAnchor),
      nextView.topAnchor.constraint(equalTo: contentHost.topAnchor),
      nextView.bottomAnchor.constraint(equalTo: contentHost.bottomAnchor),
    ])
    contentHost.layoutIfNeeded()

    let direction: CGFloat = isExpanding ? 34 : -34
    nextView.alpha = UIAccessibility.isReduceMotionEnabled ? 0 : 0.01
    nextView.transform =
      UIAccessibility.isReduceMotionEnabled
      ? .identity
      : CGAffineTransform(translationX: direction, y: 0).scaledBy(
        x: 0.97,
        y: 0.97
      )
    visibleContentView = nextView
    surface = nextSurface

    let duration = UIAccessibility.isReduceMotionEnabled ? 0.16 : 0.36
    UIView.animate(
      withDuration: duration,
      delay: 0,
      usingSpringWithDamping: 0.86,
      initialSpringVelocity: 0.18,
      options: [.beginFromCurrentState, .allowUserInteraction]
    ) {
      self.preferredContentSize = targetSize
      if let popover = self.popoverPresentationController,
        let sourceView = popover.sourceView
      {
        popover.sourceRect = sourceView.bounds.offsetBy(
          dx: isExpanding ? self.expandedHorizontalOffset : 0,
          dy: isExpanding ? self.expandedVerticalOffset : 0
        )
      }
      previousView?.alpha = 0
      previousView?.transform =
        UIAccessibility.isReduceMotionEnabled
        ? .identity
        : CGAffineTransform(translationX: -direction, y: 0).scaledBy(
          x: 0.97,
          y: 0.97
        )
      nextView.alpha = 1
      nextView.transform = .identity
      self.view.layoutIfNeeded()
    } completion: { _ in
      previousView?.removeFromSuperview()
      previousView?.transform = .identity
      completion?()
    }
  }

  private func makeGlassControl(
    title: String?,
    symbol: String?,
    accessibilityLabel: String,
    prominent: Bool = false,
    action: UIAction
  ) -> UIButton {
    var configuration = prominent
      ? UIButton.Configuration.prominentGlass()
      : UIButton.Configuration.glass()
    configuration.title = title
    if prominent {
      configuration.baseBackgroundColor = .black
    }
    if let symbol {
      configuration.image = UIImage(systemName: symbol)
    }
    configuration.imagePadding = 8
    configuration.baseForegroundColor = .white
    configuration.contentInsets = NSDirectionalEdgeInsets(
      top: 11,
      leading: 15,
      bottom: 11,
      trailing: 15
    )
    let button = UIButton(configuration: configuration, primaryAction: action)
    button.accessibilityLabel = accessibilityLabel
    return button
  }

  private func makeCameraCaptureButton() -> UIButton {
    let button = UIButton(
      primaryAction: UIAction { [weak self] _ in self?.capturePhoto() }
    )
    button.accessibilityLabel = "Take photo"
    button.translatesAutoresizingMaskIntoConstraints = false
    button.backgroundColor = UIColor.white.withAlphaComponent(0.22)
    button.layer.cornerRadius = 34
    button.layer.borderColor = UIColor.white.cgColor
    button.layer.borderWidth = 3
    let inner = UIView()
    inner.isUserInteractionEnabled = false
    inner.translatesAutoresizingMaskIntoConstraints = false
    inner.backgroundColor = .white
    inner.layer.cornerRadius = 25
    button.addSubview(inner)
    NSLayoutConstraint.activate([
      button.widthAnchor.constraint(equalToConstant: 68),
      button.heightAnchor.constraint(equalToConstant: 68),
      inner.widthAnchor.constraint(equalToConstant: 50),
      inner.heightAnchor.constraint(equalToConstant: 50),
      inner.centerXAnchor.constraint(equalTo: button.centerXAnchor),
      inner.centerYAnchor.constraint(equalTo: button.centerYAnchor),
    ])
    return button
  }

  private func addBottomControls(
    to container: UIView,
    leading: UIButton,
    center: UIButton? = nil,
    trailing: UIButton? = nil
  ) {
    leading.translatesAutoresizingMaskIntoConstraints = false
    container.addSubview(leading)
    var constraints = [
      leading.leadingAnchor.constraint(
        equalTo: container.leadingAnchor,
        constant: 12
      ),
      leading.bottomAnchor.constraint(
        equalTo: container.bottomAnchor,
        constant: -12
      ),
      leading.heightAnchor.constraint(greaterThanOrEqualToConstant: 44),
    ]

    if let center {
      center.translatesAutoresizingMaskIntoConstraints = false
      container.addSubview(center)
      constraints.append(
        center.centerXAnchor.constraint(equalTo: container.centerXAnchor)
      )
      constraints.append(
        center.bottomAnchor.constraint(
          equalTo: container.bottomAnchor,
          constant: -12
        )
      )
    }
    if let trailing {
      trailing.translatesAutoresizingMaskIntoConstraints = false
      container.addSubview(trailing)
      constraints.append(
        trailing.trailingAnchor.constraint(
          equalTo: container.trailingAnchor,
          constant: -12
        )
      )
      constraints.append(
        trailing.bottomAnchor.constraint(
          equalTo: container.bottomAnchor,
          constant: -12
        )
      )
      constraints.append(
        trailing.heightAnchor.constraint(greaterThanOrEqualToConstant: 44)
      )
    }
    NSLayoutConstraint.activate(constraints)
  }

  func picker(
    _ picker: PHPickerViewController,
    didFinishPicking results: [PHPickerResult]
  ) {
    selectionGeneration += 1
    let generation = selectionGeneration
    selectedPhotoPaths = []
    updatePhotoAction(count: results.count, preparing: !results.isEmpty)

    guard !results.isEmpty else { return }
    Task {
      do {
        var paths: [String] = []
        for result in results {
          paths.append(try await exportPickerResult(result))
        }
        await MainActor.run {
          guard generation == self.selectionGeneration else { return }
          self.selectedPhotoPaths = paths
          self.updatePhotoAction(count: paths.count, preparing: false)
        }
      } catch {
        await MainActor.run {
          guard generation == self.selectionGeneration else { return }
          self.selectedPhotoPaths = []
          self.updatePhotoAction(count: 0, preparing: false)
          self.showError("Unable to prepare the selected photos.")
        }
      }
    }
  }

  private func updatePhotoAction(count: Int, preparing: Bool) {
    let title: String
    if preparing {
      title = "Preparing…"
    } else if count == 0 {
      title = "All Photos"
    } else {
      title = "Add \(count) \(count == 1 ? "photo" : "photos")"
    }
    guard let button = photoActionButton else { return }
    let canInteract = !preparing
    button.configuration?.title = title
    button.accessibilityLabel = title
    button.isUserInteractionEnabled = canInteract
    if canInteract {
      button.accessibilityTraits.remove(.notEnabled)
    } else {
      button.accessibilityTraits.insert(.notEnabled)
    }
  }

  private func performPhotoAction() {
    if selectedPhotoPaths.isEmpty {
      finish(method: "pickAllPhotos")
    } else {
      finish(method: "photosSelected", arguments: selectedPhotoPaths)
    }
  }

  private func exportPickerResult(_ result: PHPickerResult) async throws
    -> String
  {
    let provider = result.itemProvider
    guard
      let typeIdentifier = provider.registeredTypeIdentifiers.first(where: {
        guard let type = UTType($0) else { return false }
        return type.conforms(to: .image)
      })
    else {
      throw NativeAttachmentPopoverError.unsupportedImage
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
          continuation.resume(
            throwing: NativeAttachmentPopoverError.missingFile
          )
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

  private func startCamera() {
    guard !cameraIsStarting else { return }
    cameraIsStarting = true

    switch AVCaptureDevice.authorizationStatus(for: .video) {
    case .authorized:
      configureAndStartCamera()
    case .notDetermined:
      AVCaptureDevice.requestAccess(for: .video) { [weak self] granted in
        guard let self else { return }
        if granted {
          self.configureAndStartCamera()
        } else {
          DispatchQueue.main.async {
            self.cameraIsStarting = false
            self.showCameraUnavailable(
              "Camera access is needed to take a photo."
            )
          }
        }
      }
    default:
      cameraIsStarting = false
      showCameraUnavailable("Camera access is needed to take a photo.")
    }
  }

  private func configureAndStartCamera() {
    cameraQueue.async { [weak self] in
      guard let self else { return }
      do {
        if !self.cameraConfigured {
          self.cameraSession.beginConfiguration()
          defer { self.cameraSession.commitConfiguration() }
          self.cameraSession.sessionPreset = .photo
          guard
            let device = AVCaptureDevice.default(
              .builtInWideAngleCamera,
              for: .video,
              position: .back
            )
          else {
            throw NativeAttachmentPopoverError.cameraUnavailable
          }
          let input = try AVCaptureDeviceInput(device: device)
          guard self.cameraSession.canAddInput(input) else {
            throw NativeAttachmentPopoverError.cameraUnavailable
          }
          self.cameraSession.addInput(input)
          guard self.cameraSession.canAddOutput(self.cameraOutput) else {
            throw NativeAttachmentPopoverError.cameraUnavailable
          }
          self.cameraSession.addOutput(self.cameraOutput)
          self.cameraConfigured = true
        }

        if !self.cameraSession.isRunning {
          self.cameraSession.startRunning()
        }
        DispatchQueue.main.async {
          self.cameraIsStarting = false
          self.installCameraPreview()
        }
      } catch {
        if self.cameraSession.isRunning {
          self.cameraSession.stopRunning()
        }
        DispatchQueue.main.async {
          self.cameraIsStarting = false
          self.showCameraUnavailable("Camera isn’t available here.")
        }
      }
    }
  }

  private func installCameraPreview() {
    guard surface == .camera, let previewView = cameraPreviewView else { return }
    previewView.viewWithTag(7001)?.removeFromSuperview()
    cameraPreviewLayer?.removeFromSuperlayer()

    let layer = AVCaptureVideoPreviewLayer(session: cameraSession)
    layer.videoGravity = .resizeAspectFill
    layer.frame = previewView.bounds
    previewView.layer.insertSublayer(layer, at: 0)
    cameraPreviewLayer = layer
  }

  private func stopCamera() {
    cameraPreviewLayer?.removeFromSuperlayer()
    cameraPreviewLayer = nil
    cameraQueue.async { [weak self] in
      guard let self, self.cameraSession.isRunning else { return }
      self.cameraSession.stopRunning()
    }
  }

  private func capturePhoto() {
    guard !cameraIsCapturing, cameraSession.isRunning else { return }
    cameraIsCapturing = true
    cameraCaptureButton?.isEnabled = false
    cameraCaptureButton?.transform = CGAffineTransform(
      scaleX: 0.92,
      y: 0.92
    )
    UIView.animate(
      withDuration: 0.12,
      delay: 0,
      options: [.beginFromCurrentState, .allowUserInteraction]
    ) {
      self.cameraCaptureButton?.transform = .identity
    }
    cameraOutput.capturePhoto(
      with: AVCapturePhotoSettings(),
      delegate: self
    )
  }

  func photoOutput(
    _ output: AVCapturePhotoOutput,
    didFinishProcessingPhoto photo: AVCapturePhoto,
    error: Error?
  ) {
    cameraIsCapturing = false
    cameraCaptureButton?.isEnabled = true
    guard error == nil, let data = photo.fileDataRepresentation() else {
      showError("Unable to capture the photo.")
      return
    }
    do {
      let destinationURL = FileManager.default.temporaryDirectory
        .appendingPathComponent(UUID().uuidString)
        .appendingPathExtension("jpg")
      try data.write(to: destinationURL, options: .atomic)
      finish(method: "cameraCaptured", arguments: destinationURL.path)
    } catch {
      showError("Unable to prepare the captured photo.")
    }
  }

  private func showCameraUnavailable(_ message: String) {
    guard let previewView = cameraPreviewView else { return }
    previewView.viewWithTag(7001)?.removeFromSuperview()
    let label = UILabel()
    label.text = message
    label.textColor = .white
    label.textAlignment = .center
    label.numberOfLines = 0
    label.translatesAutoresizingMaskIntoConstraints = false
    previewView.addSubview(label)
    NSLayoutConstraint.activate([
      label.centerXAnchor.constraint(equalTo: previewView.centerXAnchor),
      label.centerYAnchor.constraint(equalTo: previewView.centerYAnchor),
      label.leadingAnchor.constraint(
        greaterThanOrEqualTo: previewView.leadingAnchor,
        constant: 28
      ),
      label.trailingAnchor.constraint(
        lessThanOrEqualTo: previewView.trailingAnchor,
        constant: -28
      ),
    ])
  }

  private func showError(_ message: String) {
    let alert = UIAlertController(
      title: nil,
      message: message,
      preferredStyle: .alert
    )
    alert.addAction(UIAlertAction(title: "OK", style: .default))
    present(alert, animated: true)
  }

  private func removePhotoPicker() {
    guard let picker = photoPickerViewController else { return }
    picker.willMove(toParent: nil)
    picker.view.removeFromSuperview()
    picker.removeFromParent()
    photoPickerViewController = nil
  }

  private func finish(method: String, arguments: Any? = nil) {
    stopCamera()
    dismiss(animated: true) { [weak self] in
      guard let self else { return }
      self.channel.invokeMethod(method, arguments: arguments)
      self.notifyDismissalIfNeeded()
    }
  }

  fileprivate func dismissAndNotify() {
    dismiss(animated: true) { [weak self] in
      self?.notifyDismissalIfNeeded()
    }
  }

  private func notifyDismissalIfNeeded() {
    guard !didNotifyDismissal else { return }
    didNotifyDismissal = true
    onDismiss?()
  }
}

private enum NativeAttachmentPopoverError: Error {
  case cameraUnavailable
  case missingFile
  case unsupportedImage
}
