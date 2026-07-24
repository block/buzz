/**
 * ASCII emoticon → unicode emoji map for composer auto-replace (Slack/Discord
 * parity). Deliberately curated to unambiguous, widely recognized emoticons —
 * this is not meant to be exhaustive, just the common ones people type without
 * thinking about it.
 */
export const EMOTICON_MAP: Record<string, string> = {
  ":-)": "🙂",
  ":)": "🙂",
  ":-(": "🙁",
  ":(": "🙁",
  ":'-(": "😢",
  ":'(": "😢",
  ":-D": "😀",
  ":D": "😀",
  ";-)": "😉",
  ";)": "😉",
  ":-P": "😛",
  ":P": "😛",
  ":-p": "😛",
  ":p": "😛",
  ":-O": "😮",
  ":O": "😮",
  ":-o": "😮",
  ":o": "😮",
  ":-|": "😐",
  ":|": "😐",
  ":-*": "😘",
  ":*": "😘",
  "B-)": "😎",
  "B)": "😎",
  XD: "😆",
  xD: "😆",
  "</3": "💔",
  "<3": "❤️",
};
