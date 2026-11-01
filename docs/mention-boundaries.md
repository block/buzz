# Mention boundaries (`@name`)

Buzz CLI/SDK mention extraction (`crates/buzz-sdk/src/mentions.rs`) opens an
`@mention` when `@` is:

1. at the start of the message, or
2. preceded by a character that is **not** an ASCII letter or digit.

That rule keeps `user@example.com` from becoming a mention (the character
before `@` is an ASCII letter) while allowing:

| Content | Mentions |
| --- | --- |
| `@Scout please review` | Scout |
| `你好 @Scout` | Scout |
| `交给@Scout处理` | Scout |
| `user@example.com` | _(none)_ |

Trailing CJK after a token is a word boundary, so `@Scout处理` resolves
`Scout` rather than requiring a space.

Tracked by [#3904](https://github.com/block/buzz/issues/3904).
