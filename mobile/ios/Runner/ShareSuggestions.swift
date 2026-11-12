import BuzzPushKit
import Flutter
import Intents

/// Donates conversations the person sends top-level messages in, so iOS can
/// offer them as share-sheet suggestions. Incoming messages are never donated.
///
/// The conversation identifier carries the real community and channel IDs.
/// iOS hands it back to the share extension, which passes it to the app.
final class ShareSuggestions {
  private let containerURL: () -> URL?

  init(containerURL: @escaping () -> URL?) {
    self.containerURL = containerURL
  }

  func attach(messenger: FlutterBinaryMessenger) {
    FlutterMethodChannel(name: "buzz/share_suggestions", binaryMessenger: messenger)
      .setMethodCallHandler { [weak self] call, result in
        guard call.method == "donate" else {
          result(FlutterMethodNotImplemented)
          return
        }
        guard let self,
          let arguments = call.arguments as? [String: Any],
          let conversationIdentifier = arguments["conversationIdentifier"] as? String,
          let displayName = arguments["displayName"] as? String,
          let recipientCount = arguments["recipientCount"] as? Int
        else {
          result(FlutterError(code: "invalid_input", message: nil, details: call.arguments))
          return
        }
        // An age restriction purges every interaction; don't add new ones.
        guard !BuzzAgeRestrictionSession.isRestricted(containerURL: self.containerURL()) else {
          result(nil)
          return
        }
        Self.interaction(
          conversationIdentifier: conversationIdentifier,
          displayName: displayName,
          recipientCount: recipientCount
        ).donate { error in
          DispatchQueue.main.async {
            result(error.map {
              FlutterError(code: "donation_failed", message: $0.localizedDescription, details: nil)
            })
          }
        }
      }
  }

  static func interaction(
    conversationIdentifier: String,
    displayName: String,
    recipientCount: Int
  ) -> INInteraction {
    let intent = INSendMessageIntent(
      recipients: nil,
      outgoingMessageType: .outgoingMessageText,
      content: nil,
      speakableGroupName: INSpeakableString(spokenPhrase: displayName),
      conversationIdentifier: conversationIdentifier,
      serviceName: "Buzz",
      sender: nil,
      attachments: nil
    )
    let metadata = INSendMessageIntentDonationMetadata()
    metadata.recipientCount = recipientCount
    intent.donationMetadata = metadata
    let interaction = INInteraction(intent: intent, response: nil)
    interaction.direction = .outgoing
    return interaction
  }
}
