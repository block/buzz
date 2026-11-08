import UIKit

/// System hard-edge treatment beneath a custom Flutter navigation header.
final class NavigationEdgeBackdrop: UIViewController {
  private let scroll = UIScrollView()
  private let header = UIView()
  private let divider = UIImageView()

  override func viewDidLoad() {
    super.viewDidLoad()
    view.backgroundColor = .clear
    scroll.isUserInteractionEnabled = false
    view.accessibilityElementsHidden = true
    scroll.backgroundColor = .clear
    scroll.contentInsetAdjustmentBehavior = .always
    scroll.scrollsToTop = false
    scroll.contentSize = CGSize(width: 1, height: 10000)
    view.addSubview(scroll)
    view.addSubview(header)
    // UIKit discovers images and controls inside the registered container.
    // A real separator gives the custom Flutter header a native edge element;
    // an empty UIView or UIControl does not participate in edge registration.
    divider.image = UIGraphicsImageRenderer(size: CGSize(width: 1, height: 1)).image { context in
      UIColor.white.setFill()
      context.fill(CGRect(x: 0, y: 0, width: 1, height: 1))
    }.withRenderingMode(.alwaysTemplate)
    divider.tintColor = .separator
    header.addSubview(divider)
    if #available(iOS 26.0, *) {
      scroll.topEdgeEffect.style = .hard
      scroll.bottomEdgeEffect.isHidden = true
      let interaction = UIScrollEdgeElementContainerInteraction()
      interaction.scrollView = scroll
      interaction.edge = .top
      header.addInteraction(interaction)
    }
  }

  func layout(in frame: CGRect, headerHeight: CGFloat, automaticTop: CGFloat) {
    // Implicit Flutter engines may register the factory before a controller
    // exists. Resolve containment after the platform view enters its hierarchy.
    if parent == nil {
      var responder: UIResponder? = view.superview
      while let current = responder {
        if let host = current as? UIViewController, host !== self {
          host.addChild(self)
          didMove(toParent: host)
          break
        }
        responder = current.next
      }
    }
    view.frame = frame
    scroll.frame = view.bounds
    scroll.contentSize = CGSize(width: frame.width, height: 10000)
    additionalSafeAreaInsets.top = max(0, headerHeight - automaticTop)
    header.frame = CGRect(x: 0, y: 0, width: frame.width, height: headerHeight)
    let pixel = 1 / max(1, view.traitCollection.displayScale)
    divider.frame = CGRect(x: 0, y: max(0, headerHeight - pixel), width: frame.width, height: pixel)
    // The transparent mirror stays scrolled so the backdrop remains visible.
    scroll.contentOffset.y = 180
    view.layoutIfNeeded()
  }
}
