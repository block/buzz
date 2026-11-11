import SwiftUI
import WidgetKit

/// Lock screen widgets that open Buzz at New chat or the Bestie's DM.
///
/// A tap opens `buzz://home-shortcut/<type>`, which the app routes through the
/// same handling as its Home Screen quick actions.
@main
struct BuzzWidgets: WidgetBundle {
  var body: some Widget {
    NewChatWidget()
    BestieWidget()
  }
}

private func shortcutURL(_ type: String) -> URL {
  URL(string: "buzz://home-shortcut/\(type)")!
}

private struct StaticEntry: TimelineEntry {
  let date = Date()
}

private struct StaticProvider: TimelineProvider {
  func placeholder(in context: Context) -> StaticEntry { StaticEntry() }
  func getSnapshot(in context: Context, completion: @escaping (StaticEntry) -> Void) {
    completion(StaticEntry())
  }
  func getTimeline(in context: Context, completion: @escaping (Timeline<StaticEntry>) -> Void) {
    completion(Timeline(entries: [StaticEntry()], policy: .never))
  }
}

struct NewChatWidget: Widget {
  var body: some WidgetConfiguration {
    StaticConfiguration(kind: "NewChatWidget", provider: StaticProvider()) { _ in
      ZStack {
        AccessoryWidgetBackground()
        Image(systemName: "square.and.pencil")
          .font(.system(size: 24, weight: .semibold))
          .accessibilityHidden(true)
      }
      .accessibilityElement()
      .accessibilityLabel("New chat")
      .widgetURL(shortcutURL("new-chat"))
      .widgetBackground()
    }
    .configurationDisplayName("New chat")
    .description("Start a new chat.")
    .supportedFamilies([.accessoryCircular])
  }
}

private struct BestieEntry: TimelineEntry {
  let date = Date()
  let state: BestieWidgetState
}

private struct BestieProvider: TimelineProvider {
  func placeholder(in context: Context) -> BestieEntry {
    BestieEntry(state: BestieWidgetState())
  }
  func getSnapshot(in context: Context, completion: @escaping (BestieEntry) -> Void) {
    completion(BestieEntry(state: .load()))
  }
  func getTimeline(in context: Context, completion: @escaping (Timeline<BestieEntry>) -> Void) {
    // The app reloads this timeline whenever the Bestie changes.
    completion(Timeline(entries: [BestieEntry(state: .load())], policy: .never))
  }
}

private struct BestieWidgetView: View {
  let state: BestieWidgetState

  var body: some View {
    ZStack {
      AccessoryWidgetBackground()
      if let data = state.avatar, let image = UIImage(data: data) {
        Image(uiImage: image)
          .resizable()
          .scaledToFill()
      } else {
        Image("ShortcutAgent")
          .renderingMode(.template)
          .resizable()
          .scaledToFit()
          .padding(14)
      }
    }
    .accessibilityElement()
    .accessibilityLabel(state.name.map { "Message \($0)" } ?? "Choose your Bestie")
    .widgetURL(shortcutURL("message-bestie"))
    .widgetBackground()
  }
}

struct BestieWidget: Widget {
  var body: some WidgetConfiguration {
    StaticConfiguration(kind: BestieWidgetState.kind, provider: BestieProvider()) { entry in
      BestieWidgetView(state: entry.state)
    }
    .configurationDisplayName("Bestie")
    .description("Message your Bestie.")
    .supportedFamilies([.accessoryCircular])
  }
}

extension View {
  /// iOS 17 requires a container background; iOS 16 draws none.
  @ViewBuilder fileprivate func widgetBackground() -> some View {
    if #available(iOS 17.0, *) {
      containerBackground(for: .widget) {}
    } else {
      self
    }
  }
}
