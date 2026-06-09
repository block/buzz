import type { UserNote } from "@/shared/api/socialTypes";

export function getReplyParent(note: UserNote): string | null {
  const eTags = note.tags.filter((tag) => tag[0] === "e" && tag[1]);
  for (let index = eTags.length - 1; index >= 0; index -= 1) {
    const tag = eTags[index];
    if (tag[3] === "reply") {
      return tag[1] ?? null;
    }
  }

  for (let index = eTags.length - 1; index >= 0; index -= 1) {
    const tag = eTags[index];
    if (tag[3] == null) {
      return tag[1] ?? null;
    }
  }

  for (let index = eTags.length - 1; index >= 0; index -= 1) {
    const tag = eTags[index];
    if (tag[3] === "root") {
      return tag[1] ?? null;
    }
  }

  return null;
}

export function noteSnippet(content: string) {
  return content.trim().replace(/\s+/g, " ").slice(0, 120);
}
