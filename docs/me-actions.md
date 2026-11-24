# Narrated message actions

Typing `/me passes out` sends a normal chat message whose signed content is
exactly `/me passes out`. Supporting clients display a compact action row with
the sender's avatar and an italicized sentence, such as *Alex passes out*.
The sender's name comes from the event author's identity in the current
community, never from a name embedded in the message.

## Recognition

Only the exact lowercase prefix `/me ` at the start of a message, followed by a
non-whitespace body, is an action. Trim surrounding whitespace from the displayed
action body without changing the saved content. `/me`, `/me   `, `/meeting`,
`/ME waves`, a quoted command, a code block, and `\/me waves` remain ordinary
messages. Actions may span multiple lines.

This is a presentation convention for existing messages, not a new event type.
Keep the existing message kind, channel and thread tags, mentions, attachments,
and delivery flow. There is deliberately no new kind: older clients retain a
readable `/me …` fallback, and search, unread counts, notifications, moderation,
edits, deletion, reactions, and replies retain normal message behavior.

## Presentation

Actions are authored messages, visually distinct from both speech bubbles and
relay-generated system notices. Show the author's avatar and name on every
action, including consecutive actions, the viewer's own messages, and DMs.
Use a small avatar and normal-size italic action text with readable contrast;
put clock time above the narration. Align the small avatar's leading edge with
ordinary message avatars and center it vertically on the narration's first line.
Keep the text to its right, with wrapped lines, reactions, and reply controls
aligned under the narration. Keep edit
markers available. Profile labels render literally.
Consecutive actions within the same minute share one visible timestamp, while
each action retains its own avatar, name, and accessible time.
Keep links, mentions, emoji, and attachments usable inside the action body.

Do not group an action with neighboring speech bubbles. Keep the usual message
menu, reply, reaction, copy, edit, and delete affordances. Editing into or out of
the `/me ` prefix changes presentation based on the latest effective content;
the edit composer and copied message retain the original authored prefix.

This does not change presence or profile status. It records something someone
narrated at a particular moment in a conversation.
