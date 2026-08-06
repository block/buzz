import AVFoundation
import Flutter
import PhotosUI
import UIKit
import UniformTypeIdentifiers

@available(iOS 26.0, *)
final class NativeAttachmentPopoverViewController:
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
  private let maximumMenuHeight: CGFloat
  private let expandedHeight = NativeAttachmentMenuLayout.maximumHeight
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
  private var cameraDevice: AVCaptureDevice?
  private var cameraRotationCoordinator: AVCaptureDevice.RotationCoordinator?
  private var cameraRotationObservation: NSKeyValueObservation?
  private var selectionGeneration = 0
  private var selectionTask: Task<Void, Never>?
  private var selectedPhotoPaths: [String] = []
  private var cameraConfigured = false
  private var cameraIsStarting = false
  private var cameraStartupGeneration = 0
  private var cameraIsCapturing = false
  private var activeCameraCaptureID: Int64?
  private var isFinishing = false
  private var didNotifyDismissal = false
  private var keyboardDismissalOffset: CGFloat = 0
  private var menuStackHeightConstraint: NSLayoutConstraint?

  var onDismiss: (() -> Void)?

  private var menuSize: CGSize {
    NativeAttachmentMenuLayout.size(
      compatibleWith: traitCollection,
      maximumHeight: maximumMenuHeight
    )
  }

  init(
    channel: FlutterMethodChannel,
    expandedWidth: CGFloat,
    maximumMenuHeight: CGFloat = NativeAttachmentMenuLayout.maximumHeight
  ) {
    self.channel = channel
    self.expandedWidth = expandedWidth
    self.maximumMenuHeight = maximumMenuHeight
    super.init(nibName: nil, bundle: nil)
    preferredContentSize = NativeAttachmentMenuLayout.size(
      compatibleWith: .current,
      maximumHeight: maximumMenuHeight
    )
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) {
    fatalError("init(coder:) has not been implemented")
  }

  override func viewDidLoad() {
    super.viewDidLoad()
    view.backgroundColor = .clear
    view.layer.cornerRadius = NativeAttachmentPopoverStyle.cornerRadius
    view.layer.cornerCurve = .continuous
    view.layer.borderColor = UIColor.black.withAlphaComponent(0.04).cgColor
    view.layer.borderWidth = NativeAttachmentPopoverStyle.borderWidth
    view.layer.shadowColor = UIColor.black.cgColor
    view.layer.shadowOpacity = NativeAttachmentPopoverStyle.shadowOpacity
    view.layer.shadowRadius = NativeAttachmentPopoverStyle.shadowRadius
    view.layer.shadowOffset = NativeAttachmentPopoverStyle.shadowOffset
    view.clipsToBounds = false

    let glassEffect = UIGlassEffect(style: .regular)
    glassEffect.isInteractive = true
    let glassView = UIVisualEffectView(effect: glassEffect)
    glassView.translatesAutoresizingMaskIntoConstraints = false
    glassView.layer.cornerRadius = NativeAttachmentPopoverStyle.cornerRadius
    glassView.layer.cornerCurve = .continuous
    glassView.clipsToBounds = true
    view.addSubview(glassView)

    contentHost.translatesAutoresizingMaskIntoConstraints = false
    contentHost.layer.cornerRadius = NativeAttachmentPopoverStyle.cornerRadius
    contentHost.layer.cornerCurve = .continuous
    contentHost.clipsToBounds = true
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
    view.layer.shadowPath =
      UIBezierPath(
        roundedRect: view.bounds,
        cornerRadius: NativeAttachmentPopoverStyle.cornerRadius
      ).cgPath
    cameraPreviewLayer?.frame = cameraPreviewView?.bounds ?? .zero
  }

  override func viewWillDisappear(_ animated: Bool) {
    super.viewWillDisappear(animated)
    invalidateCameraCapture()
    stopCamera()
  }

  override func traitCollectionDidChange(
    _ previousTraitCollection: UITraitCollection?
  ) {
    super.traitCollectionDidChange(previousTraitCollection)
    guard
      previousTraitCollection?.preferredContentSizeCategory
        != traitCollection.preferredContentSizeCategory
    else {
      return
    }

    updateMenuLayout()
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

    let scrollView = UIScrollView()
    scrollView.alwaysBounceVertical = false
    scrollView.translatesAutoresizingMaskIntoConstraints = false
    container.addSubview(scrollView)

    let stack = UIStackView()
    stack.axis = .vertical
    stack.distribution = .fillEqually
    stack.spacing = NativeAttachmentMenuLayout.itemSpacing
    stack.translatesAutoresizingMaskIntoConstraints = false
    scrollView.addSubview(stack)
    let stackHeightConstraint = stack.heightAnchor.constraint(
      equalToConstant: NativeAttachmentMenuLayout.itemsHeight(
        compatibleWith: traitCollection
      )
    )
    menuStackHeightConstraint = stackHeightConstraint
    NSLayoutConstraint.activate([
      scrollView.leadingAnchor.constraint(equalTo: container.leadingAnchor),
      scrollView.trailingAnchor.constraint(equalTo: container.trailingAnchor),
      scrollView.topAnchor.constraint(equalTo: container.topAnchor),
      scrollView.bottomAnchor.constraint(equalTo: container.bottomAnchor),
      stack.leadingAnchor.constraint(
        equalTo: scrollView.frameLayoutGuide.leadingAnchor,
        constant: NativeAttachmentMenuLayout.contentPadding
      ),
      stack.trailingAnchor.constraint(
        equalTo: scrollView.frameLayoutGuide.trailingAnchor,
        constant: -NativeAttachmentMenuLayout.contentPadding
      ),
      stack.topAnchor.constraint(
        equalTo: scrollView.contentLayoutGuide.topAnchor,
        constant: NativeAttachmentMenuLayout.contentPadding
      ),
      stack.bottomAnchor.constraint(
        equalTo: scrollView.contentLayoutGuide.bottomAnchor,
        constant: -NativeAttachmentMenuLayout.contentPadding
      ),
      stackHeightConstraint,
    ])

    stack.addArrangedSubview(
      makeNativeAttachmentMenuButton(
        title: "Camera",
        symbol: "camera",
        action: { [weak self] in self?.showCamera() }
      )
    )
    stack.addArrangedSubview(
      makeNativeAttachmentMenuButton(
        title: "Photos",
        symbol: "photo.on.rectangle.angled",
        action: { [weak self] in self?.showPhotos() }
      )
    )
    stack.addArrangedSubview(
      makeNativeAttachmentMenuButton(
        title: "Video",
        symbol: "video",
        action: { [weak self] in
          self?.finish(method: "pickVideo")
        }
      )
    )
    stack.addArrangedSubview(
      makeNativeAttachmentMenuButton(
        title: "Files",
        symbol: "doc",
        action: { [weak self] in
          self?.finish(method: "pickFiles")
        }
      )
    )
    return container
  }

  private func updateMenuLayout() {
    menuStackHeightConstraint?.constant =
      NativeAttachmentMenuLayout.itemsHeight(
        compatibleWith: traitCollection
      )
    if surface == .menu {
      preferredContentSize = menuSize
    }
  }

  private func showPhotos() {
    guard surface != .photos else { return }
    prepareForExpandedSurface()
    stopCamera()

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

    let container = UIView()
    container.backgroundColor = .clear
    container.clipsToBounds = true
    addChild(picker)
    picker.view.translatesAutoresizingMaskIntoConstraints = false
    container.addSubview(picker.view)
    NSLayoutConstraint.activate([
      picker.view.leadingAnchor.constraint(equalTo: container.leadingAnchor),
      picker.view.trailingAnchor.constraint(equalTo: container.trailingAnchor),
      picker.view.topAnchor.constraint(equalTo: container.topAnchor),
      picker.view.bottomAnchor.constraint(equalTo: container.bottomAnchor),
    ])
    picker.didMove(toParent: self)
    photoPickerViewController = picker

    let backButton = makeGlassControl(
      title: nil,
      symbol: "chevron.left",
      accessibilityLabel: "Back to attachment options",
      action: { [weak self] in self?.showMenu() }
    )
    let actionButton = makeGlassControl(
      title: "All Photos",
      symbol: nil,
      accessibilityLabel: "All Photos",
      prominent: true,
      action: { [weak self] in self?.performPhotoAction() }
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
    prepareForExpandedSurface()
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
      action: { [weak self] in self?.showMenu() }
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
    invalidateCameraCapture()
    selectionGeneration += 1
    selectionTask?.cancel()
    selectionTask = nil
    Self.removeTemporaryFiles(selectedPhotoPaths)
    selectedPhotoPaths = []
    stopCamera()

    let menu = makeMenuView()
    transition(
      to: .menu,
      content: menu,
      completion: { [weak self] in
        self?.removePhotoPicker()
      }
    )
  }

  private func prepareForExpandedSurface() {
    if let sourceHost = popoverPresentationController?.sourceView?.superview {
      keyboardDismissalOffset = max(
        keyboardDismissalOffset,
        NativeAttachmentExpandedSurfaceBehavior.keyboardOverlap(
          containerBounds: sourceHost.bounds,
          keyboardLayoutFrame: sourceHost.keyboardLayoutGuide.layoutFrame
        )
      )
    }
    NativeAttachmentExpandedSurfaceBehavior.dismissKeyboard(in: view.window)
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

    let shouldAnimate = !UIAccessibility.isReduceMotionEnabled
    // Do not expose embedded surfaces while the popover changes size. In
    // particular, PHPicker visibly reflows its grid from the compact menu
    // width to the expanded width if it is allowed to paint during this step.
    nextView.alpha = 0
    visibleContentView = nextView
    surface = nextSurface

    let duration = shouldAnimate ? (isExpanding ? 0.24 : 0.2) : 0
    UIView.animate(
      withDuration: duration,
      delay: 0,
      options: [
        .beginFromCurrentState,
        .allowUserInteraction,
        .curveEaseInOut,
      ]
    ) {
      self.preferredContentSize = targetSize
      if let popover = self.popoverPresentationController,
        let sourceView = popover.sourceView
      {
        popover.sourceRect = NativeAttachmentPopoverAnchorLayout.sourceRect(
          anchorBounds: sourceView.bounds,
          keyboardDismissalOffset: self.keyboardDismissalOffset,
          isExpanded: isExpanding
        )
      }
      previousView?.alpha = 0
      self.view.layoutIfNeeded()
    } completion: { _ in
      previousView?.removeFromSuperview()
      self.view.layoutIfNeeded()

      let reveal = {
        UIView.animate(
          withDuration: shouldAnimate ? 0.14 : 0,
          delay: 0,
          options: [
            .beginFromCurrentState,
            .allowUserInteraction,
            .curveEaseOut,
          ]
        ) {
          nextView.alpha = 1
        } completion: { _ in
          completion?()
        }
      }

      reveal()
    }
  }

  private func makeGlassControl(
    title: String?,
    symbol: String?,
    accessibilityLabel: String,
    prominent: Bool = false,
    action: @escaping () -> Void
  ) -> UIButton {
    var configuration =
      prominent
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
    configuration.titleTextAttributesTransformer =
      UIConfigurationTextAttributesTransformer { attributes in
        var interAttributes = attributes
        interAttributes.font = NativeAttachmentMenuTypography.font(
          forTextStyle: .body
        )
        return interAttributes
      }
    configuration.contentInsets = NSDirectionalEdgeInsets(
      top: 11,
      leading: 15,
      bottom: 11,
      trailing: 15
    )
    let button = UIButton(
      configuration: configuration,
      primaryAction: UIAction { _ in
        UISelectionFeedbackGenerator().selectionChanged()
        action()
      }
    )
    button.accessibilityLabel = accessibilityLabel
    return button
  }

  private func makeCameraCaptureButton() -> UIButton {
    let button = UIButton(
      primaryAction: UIAction { [weak self] _ in
        UISelectionFeedbackGenerator().selectionChanged()
        self?.capturePhoto()
      }
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
    selectionTask?.cancel()
    selectionTask = nil
    Self.removeTemporaryFiles(selectedPhotoPaths)
    selectedPhotoPaths = []
    updatePhotoAction(count: results.count, preparing: !results.isEmpty)

    guard !results.isEmpty else { return }
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
          self.selectedPhotoPaths = paths
          self.selectionTask = nil
          self.updatePhotoAction(count: paths.count, preparing: false)
        }
      } catch is CancellationError {
        Self.removeTemporaryFiles(paths)
      } catch {
        Self.removeTemporaryFiles(paths)
        await MainActor.run {
          guard generation == self.selectionGeneration else { return }
          self.selectionTask = nil
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
      let paths = selectedPhotoPaths
      selectedPhotoPaths = []
      finish(
        method: "photosSelected",
        arguments: paths,
        temporaryPaths: paths
      )
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

  private static func removeTemporaryFiles(_ paths: [String]) {
    for path in paths where !path.isEmpty {
      try? FileManager.default.removeItem(atPath: path)
    }
  }

  private func startCamera() {
    guard !cameraIsStarting else { return }
    cameraIsStarting = true
    cameraStartupGeneration += 1
    let startupGeneration = cameraStartupGeneration

    switch AVCaptureDevice.authorizationStatus(for: .video) {
    case .authorized:
      configureAndStartCamera(startupGeneration: startupGeneration)
    case .notDetermined:
      AVCaptureDevice.requestAccess(for: .video) { [weak self] granted in
        DispatchQueue.main.async {
          guard
            let self,
            self.cameraStartupIsCurrent(startupGeneration)
          else { return }
          if granted {
            self.configureAndStartCamera(
              startupGeneration: startupGeneration
            )
          } else {
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

  private func configureAndStartCamera(startupGeneration: Int) {
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
          self.cameraDevice = device
          guard self.cameraSession.canAddOutput(self.cameraOutput) else {
            throw NativeAttachmentPopoverError.cameraUnavailable
          }
          self.cameraSession.addOutput(self.cameraOutput)
          self.cameraConfigured = true
        }

        if !self.cameraSession.isRunning {
          self.cameraSession.startRunning()
        }
        DispatchQueue.main.async { [weak self] in
          guard
            let self,
            self.cameraStartupIsCurrent(startupGeneration)
          else { return }
          self.cameraIsStarting = false
          self.installCameraPreview()
        }
      } catch {
        if self.cameraSession.isRunning {
          self.cameraSession.stopRunning()
        }
        DispatchQueue.main.async { [weak self] in
          guard
            let self,
            self.cameraStartupIsCurrent(startupGeneration)
          else { return }
          self.cameraIsStarting = false
          self.showCameraUnavailable("Camera isn’t available here.")
        }
      }
    }
  }

  private func cameraStartupIsCurrent(_ generation: Int) -> Bool {
    generation == cameraStartupGeneration && surface == .camera && !isFinishing
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

    if let cameraDevice {
      let coordinator = AVCaptureDevice.RotationCoordinator(
        device: cameraDevice,
        previewLayer: layer
      )
      cameraRotationCoordinator = coordinator
      cameraRotationObservation = coordinator.observe(
        \.videoRotationAngleForHorizonLevelPreview,
        options: [.initial, .new]
      ) { [weak layer] coordinator, _ in
        guard let connection = layer?.connection else { return }
        let angle = coordinator.videoRotationAngleForHorizonLevelPreview
        guard connection.isVideoRotationAngleSupported(angle) else { return }
        connection.videoRotationAngle = angle
      }
    }
  }

  private func stopCamera() {
    cameraStartupGeneration += 1
    cameraIsStarting = false
    cameraRotationObservation?.invalidate()
    cameraRotationObservation = nil
    cameraRotationCoordinator = nil
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
    if let connection = cameraOutput.connection(with: .video),
      let cameraRotationCoordinator
    {
      let angle =
        cameraRotationCoordinator.videoRotationAngleForHorizonLevelCapture
      if connection.isVideoRotationAngleSupported(angle) {
        connection.videoRotationAngle = angle
      }
    }
    let settings = AVCapturePhotoSettings()
    activeCameraCaptureID = settings.uniqueID
    cameraOutput.capturePhoto(with: settings, delegate: self)
  }

  func photoOutput(
    _ output: AVCapturePhotoOutput,
    didFinishProcessingPhoto photo: AVCapturePhoto,
    error: Error?
  ) {
    guard error == nil, let data = photo.fileDataRepresentation() else {
      DispatchQueue.main.async { [weak self] in
        self?.completeCameraCapture(
          captureID: photo.resolvedSettings.uniqueID,
          path: nil,
          errorMessage: "Unable to capture the photo."
        )
      }
      return
    }

    DispatchQueue.global(qos: .userInitiated).async {
      do {
        let destinationURL = FileManager.default.temporaryDirectory
          .appendingPathComponent(UUID().uuidString)
          .appendingPathExtension("jpg")
        try data.write(to: destinationURL, options: .atomic)
        DispatchQueue.main.async { [weak self] in
          guard let self else {
            Self.removeTemporaryFiles([destinationURL.path])
            return
          }
          self.completeCameraCapture(
            captureID: photo.resolvedSettings.uniqueID,
            path: destinationURL.path,
            errorMessage: nil
          )
        }
      } catch {
        DispatchQueue.main.async { [weak self] in
          self?.completeCameraCapture(
            captureID: photo.resolvedSettings.uniqueID,
            path: nil,
            errorMessage: "Unable to prepare the captured photo."
          )
        }
      }
    }
  }

  @MainActor
  private func completeCameraCapture(
    captureID: Int64,
    path: String?,
    errorMessage: String?
  ) {
    guard
      captureID == activeCameraCaptureID, surface == .camera, !isFinishing
    else {
      if let path { Self.removeTemporaryFiles([path]) }
      return
    }
    activeCameraCaptureID = nil
    cameraIsCapturing = false
    cameraCaptureButton?.isEnabled = true
    if let path {
      finish(
        method: "cameraCaptured",
        arguments: path,
        temporaryPaths: [path]
      )
    } else if !isFinishing, let errorMessage {
      showError(errorMessage)
    }
  }

  private func invalidateCameraCapture() {
    activeCameraCaptureID = nil
    cameraIsCapturing = false
    cameraCaptureButton?.isEnabled = true
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

  private func finish(
    method: String,
    arguments: Any? = nil,
    temporaryPaths: [String] = []
  ) {
    guard !isFinishing else {
      Self.removeTemporaryFiles(temporaryPaths)
      return
    }
    NativeAttachmentExpandedSurfaceBehavior.dismissKeyboard(in: view.window)
    isFinishing = true
    view.isUserInteractionEnabled = false
    selectionGeneration += 1
    selectionTask?.cancel()
    selectionTask = nil
    stopCamera()
    dismiss(animated: true) { [weak self] in
      guard let self else {
        Self.removeTemporaryFiles(temporaryPaths)
        return
      }
      self.channel.invokeMethod(method, arguments: arguments) { _ in
        Self.removeTemporaryFiles(temporaryPaths)
      }
      self.notifyDismissalIfNeeded()
    }
  }

  func dismissAndNotify() {
    guard !isFinishing else { return }
    isFinishing = true
    view.isUserInteractionEnabled = false
    selectionGeneration += 1
    selectionTask?.cancel()
    selectionTask = nil
    Self.removeTemporaryFiles(selectedPhotoPaths)
    selectedPhotoPaths = []
    stopCamera()
    dismiss(animated: true) { [weak self] in
      self?.notifyDismissalIfNeeded()
    }
  }

  private func notifyDismissalIfNeeded() {
    selectionGeneration += 1
    selectionTask?.cancel()
    selectionTask = nil
    Self.removeTemporaryFiles(selectedPhotoPaths)
    selectedPhotoPaths = []
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
