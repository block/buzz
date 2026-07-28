import ActivityKit
import SwiftUI
import UIKit
import WidgetKit

@main
struct AgentActivityWidgetBundle: WidgetBundle {
  var body: some Widget {
    AgentActivityLiveActivity()
  }
}

struct AgentActivityLiveActivity: Widget {
  var body: some WidgetConfiguration {
    ActivityConfiguration(for: AgentActivityAttributes.self) { context in
      AgentActivityLockScreenView(state: context.state)
        .activityBackgroundTint(.black)
        .activitySystemActionForegroundColor(.white)
        .widgetURL(URL(string: context.state.deepLink))
    } dynamicIsland: { context in
      DynamicIsland {
        DynamicIslandExpandedRegion(.leading) {
          BuzzLogo(size: 36)
        }
        DynamicIslandExpandedRegion(.trailing) {
          Text(context.state.startedAt, style: .timer)
            .font(.caption.monospacedDigit())
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.trailing)
            .padding(.trailing, 8)
            .frame(width: 100, height: 36, alignment: .trailing)
        }
        DynamicIslandExpandedRegion(.bottom) {
          VStack(alignment: .leading, spacing: 3) {
            AgentActivityTitle(state: context.state)
              .font(.headline)
              .lineLimit(1)
            AgentActivityBody(state: context.state)
              .font(.subheadline)
              .foregroundStyle(.secondary)
              .lineLimit(2)
          }
          .multilineTextAlignment(.leading)
          .frame(maxWidth: .infinity, alignment: .leading)
          .padding(.top, 6)
          .padding(.bottom, 8)
        }
      } compactLeading: {
        BuzzLogo(size: 22)
      } compactTrailing: {
        Text("\(context.state.activeAgentCount)")
          .font(.caption2.bold().monospacedDigit())
          .foregroundStyle(.white)
          .accessibilityLabel(
            "\(context.state.activeAgentCount) active agents"
          )
      } minimal: {
        BuzzLogo(size: 22)
      }
      .keylineTint(.white)
      .widgetURL(URL(string: context.state.deepLink))
    }
  }
}

private struct AgentActivityLockScreenView: View {
  let state: AgentActivityAttributes.ContentState

  var body: some View {
    HStack(alignment: .top, spacing: 12) {
      BuzzLogo(size: 36)
      VStack(alignment: .leading, spacing: 3) {
        AgentActivityTitle(state: state)
          .font(.headline)
          .lineLimit(2)
        AgentActivityBody(state: state)
          .font(.subheadline)
          .foregroundStyle(.secondary)
          .lineLimit(2)
          .fixedSize(horizontal: false, vertical: true)
      }
      .frame(maxWidth: .infinity, alignment: .leading)
      .layoutPriority(1)
      Text(state.startedAt, style: .timer)
        .font(.caption.monospacedDigit())
        .foregroundStyle(.white)
        .frame(minWidth: 48, minHeight: 36, alignment: .trailing)
        .layoutPriority(2)
        .accessibilityLabel("Agent activity elapsed time")
    }
    .frame(maxWidth: .infinity, alignment: .leading)
    .padding(16)
  }
}

private struct BuzzLogo: View {
  let size: CGFloat

  var body: some View {
    Group {
      if let image = widgetSafeBuzzLogo {
        Image(uiImage: image)
          .renderingMode(.original)
          .resizable()
          .scaledToFit()
      }
    }
    .frame(width: size, height: size)
    .accessibilityLabel("Buzz agent activity")
  }
}

private let widgetSafeBuzzLogo: UIImage? = {
  guard
    let imageURL = Bundle.main.url(
      forResource: "buzz-icon",
      withExtension: "png"
    ),
    let sourceImage = UIImage(contentsOfFile: imageURL.path)
  else {
    return nil
  }

  let size = CGSize(width: 36, height: 24)
  let format = UIGraphicsImageRendererFormat()
  format.opaque = false
  format.scale = 3
  return UIGraphicsImageRenderer(size: size, format: format).image { _ in
    sourceImage.draw(in: CGRect(origin: .zero, size: size))
  }
}()

private struct AgentActivityTitle: View {
  @Environment(\.redactionReasons) private var redactionReasons

  let state: AgentActivityAttributes.ContentState

  var body: some View {
    if redactionReasons.contains(.privacy) {
      Text(state.publicTitle)
        .unredacted()
    } else {
      Text(state.title)
        .privacySensitive()
    }
  }
}

private struct AgentActivityBody: View {
  @Environment(\.redactionReasons) private var redactionReasons

  let state: AgentActivityAttributes.ContentState

  var body: some View {
    if redactionReasons.contains(.privacy) {
      Text(state.publicBody)
        .unredacted()
    } else {
      Text(state.body)
        .privacySensitive()
    }
  }
}
