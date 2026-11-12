/// The emoji that earn a particle burst.
///
/// Ported verbatim from `POSITIVE_EMOJI_PARTICLES` in
/// `desktop/src/shared/ui/EmojiBurstProvider.tsx`. Only affirmation reacts
/// celebrate — a 😢 or 👎 shouldn't throw confetti — so the three groups below
/// are the whole gate, and both clients must agree on them or the same reaction
/// bursts on one device and not the other.
library;

const heartParticleEmojis = <String>[
  '\u{2764}\u{FE0F}', // ❤️
  '\u{1FA77}', // 🩷
  '\u{1F9E1}', // 🧡
  '\u{1F49B}', // 💛
  '\u{1F49A}', // 💚
  '\u{1FA75}', // 🩵
  '\u{1F499}', // 💙
  '\u{1F49C}', // 💜
  '\u{1F90E}', // 🤎
  '\u{1F5A4}', // 🖤
  '\u{1FA76}', // 🩶
  '\u{1F90D}', // 🤍
  '\u{2764}\u{FE0F}\u{200D}\u{1F525}', // ❤️‍🔥
  '\u{2764}\u{FE0F}\u{200D}\u{1FA79}', // ❤️‍🩹
  '\u{1F496}', // 💖
  '\u{1F495}', // 💕
  '\u{1F497}', // 💗
  '\u{1F493}', // 💓
  '\u{1F49E}', // 💞
  '\u{1F498}', // 💘
  '\u{1F49D}', // 💝
  '\u{2763}\u{FE0F}', // ❣️
  '\u{2665}\u{FE0F}', // ♥️
];

const positiveReactionParticleEmojis = <String>[
  '\u{1F44D}', // 👍
  '\u{1F44F}', // 👏
  '\u{1F64C}', // 🙌
  '\u{1F64F}', // 🙏
  '\u{1F4AF}', // 💯
  '\u{1F525}', // 🔥
  '\u{2728}', // ✨
  '\u{2B50}', // ⭐
  '\u{1F31F}', // 🌟
  '\u{1F389}', // 🎉
  '\u{1F38A}', // 🎊
  '\u{1F973}', // 🥳
  '\u{1F680}', // 🚀
  '\u{1F4AA}', // 💪
];

const positiveFaceParticleEmojis = <String>[
  '\u{1F600}', // 😀
  '\u{1F603}', // 😃
  '\u{1F604}', // 😄
  '\u{1F601}', // 😁
  '\u{1F60A}', // 😊
  '\u{1F607}', // 😇
  '\u{1F642}', // 🙂
  '\u{1F60D}', // 😍
  '\u{1F929}', // 🤩
  '\u{1F970}', // 🥰
  '\u{1F618}', // 😘
  '\u{1F60B}', // 😋
  '\u{1F60E}', // 😎
  '\u{1F602}', // 😂
  '\u{1F923}', // 🤣
];

const positiveEmojiParticles = <String>[
  ...heartParticleEmojis,
  ...positiveReactionParticleEmojis,
  ...positiveFaceParticleEmojis,
];

/// Emoji presentation selectors carry no meaning for this comparison, and the
/// same emoji reaches us with and without one depending on where it came from
/// (the emoji-mart dataset, a hand-typed reaction, an older client). Desktop
/// gets away with exact set membership because its picker is the only producer;
/// mobile reactions also arrive off the wire, so fold the selector away.
String _foldSelectors(String emoji) =>
    emoji.trim().replaceAll('\u{FE0F}', '').replaceAll('\u{FE0E}', '');

final Set<String> _positiveEmojiParticleSet = {
  for (final emoji in positiveEmojiParticles) _foldSelectors(emoji),
};

/// Whether [emoji] should burst. Mirrors desktop's `isPositiveEmojiParticle`.
bool isPositiveEmojiParticle(String emoji) =>
    _positiveEmojiParticleSet.contains(_foldSelectors(emoji));
