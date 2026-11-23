import Foundation

/// Verified local values used to specialize an ordinary notification as communication.
public struct BuzzCommunicationNotificationDescriptor: Equatable, Sendable {
  public let senderDisplayName: String?
  public let senderIdentifier: String
  public let senderAvatarPNG: Data?
  public let messageBody: String
  public let conversationIdentifier: String
  public let conversationDisplayName: String?
  /// Verified recipients represented by the incoming message, excluding its sender.
  public let recipientCount: Int

  public init(
    senderDisplayName: String?,
    senderIdentifier: String,
    senderAvatarPNG: Data?,
    messageBody: String,
    conversationIdentifier: String,
    conversationDisplayName: String?,
    recipientCount: Int
  ) {
    self.senderDisplayName = senderDisplayName
    self.senderIdentifier = senderIdentifier
    self.senderAvatarPNG = senderAvatarPNG
    self.messageBody = messageBody
    self.conversationIdentifier = conversationIdentifier
    self.conversationDisplayName = conversationDisplayName
    self.recipientCount = recipientCount
  }

  public init?(resolution: BuzzPushResolution) {
    guard let target = resolution.navigationTarget,
      let senderPubkey = resolution.senderPubkey,
      !senderPubkey.isEmpty,
      let conversationIdentifier = resolution.conversationIdentifier,
      !conversationIdentifier.isEmpty,
      let recipientCount = resolution.conversationRecipientCount,
      recipientCount > 0
    else { return nil }
    self.init(
      senderDisplayName: resolution.title.isEmpty ? nil : resolution.title,
      senderIdentifier: BuzzPushPresentationIdentity.sender(
        communityID: target.communityID,
        pubkey: senderPubkey
      ),
      senderAvatarPNG: resolution.senderAvatarPNG,
      messageBody: resolution.body,
      conversationIdentifier: conversationIdentifier,
      conversationDisplayName: resolution.conversationDisplayName,
      recipientCount: recipientCount
    )
  }
}

#if os(iOS)
  import Intents
  import UserNotifications

  /// Applies Apple's Communication Notifications intent to an incoming message.
  ///
  /// The intent is not donated: iOS renders the sender avatar and app badge
  /// from the updated content alone, and share-sheet suggestions are learned
  /// only from messages the person sends (see CONTEXT.md "Share target").
  public final class BuzzCommunicationNotificationPresenter {
    public typealias ContentUpdate = (
      UNMutableNotificationContent,
      INSendMessageIntent
    ) throws -> UNNotificationContent

    private let updateContent: ContentUpdate

    public init(
      updateContent: @escaping ContentUpdate = { content, intent in
        try content.updating(from: intent)
      }
    ) {
      self.updateContent = updateContent
    }

    /// Returns communication content, or the ordinary content when the
    /// resolution lacks a verified sender and conversation.
    public func present(
      ordinaryContent: UNMutableNotificationContent,
      resolution: BuzzPushResolution
    ) -> UNNotificationContent {
      guard let descriptor = BuzzCommunicationNotificationDescriptor(resolution: resolution)
      else { return ordinaryContent }
      do {
        return try updateContent(ordinaryContent, Self.makeIntent(descriptor))
      } catch {
        NSLog("Communication notification update failed: %@", String(describing: error))
        return ordinaryContent
      }
    }

    public static func makeIntent(
      _ descriptor: BuzzCommunicationNotificationDescriptor
    ) -> INSendMessageIntent {
      let senderAvatar = descriptor.senderAvatarPNG.map(INImage.init(imageData:))
      let sender = INPerson(
        personHandle: INPersonHandle(value: descriptor.senderIdentifier, type: .unknown),
        nameComponents: nil,
        displayName: descriptor.senderDisplayName,
        image: senderAvatar,
        contactIdentifier: nil,
        customIdentifier: descriptor.senderIdentifier,
        isMe: false,
        suggestionType: .none
      )
      let intent = INSendMessageIntent(
        recipients: nil,
        outgoingMessageType: .outgoingMessageText,
        content: descriptor.messageBody,
        speakableGroupName: descriptor.conversationDisplayName.map {
          INSpeakableString(spokenPhrase: $0)
        },
        conversationIdentifier: descriptor.conversationIdentifier,
        serviceName: "Buzz",
        sender: sender,
        attachments: nil
      )
      if descriptor.conversationDisplayName != nil {
        let donationMetadata = INSendMessageIntentDonationMetadata()
        donationMetadata.recipientCount = descriptor.recipientCount
        intent.donationMetadata = donationMetadata
        if let senderAvatar {
          // Communication Notifications render a group conversation's image
          // from the speakable-group parameter rather than INPerson.image.
          // Buzz channels do not have a separate avatar, so use the verified
          // sender thumbnail for the visible incoming-message avatar.
          intent.setImage(senderAvatar, forParameterNamed: \.speakableGroupName)
        }
      }
      return intent
    }
  }
#endif
