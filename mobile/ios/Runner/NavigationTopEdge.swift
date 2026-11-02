import UIKit

/// Applies the app's soft navigation edge to app-owned native sheets.
/// OS-owned pickers retain their system presentation.
enum NavigationTopEdge {
  static func configure(_ scrollView: UIScrollView, item: UINavigationItem) {
    if #available(iOS 26.0, *) {
      scrollView.topEdgeEffect.style = .soft
      scrollView.bottomEdgeEffect.isHidden = true
    }
    let appearance = UINavigationBarAppearance()
    appearance.configureWithDefaultBackground()
    item.standardAppearance = appearance
    item.scrollEdgeAppearance = appearance
    item.compactAppearance = appearance
    item.compactScrollEdgeAppearance = appearance
  }
}
