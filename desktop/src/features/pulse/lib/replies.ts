import type { UserNote } from "@/shared/api/socialTypes";
import { truncateByCharacters } from "@/shared/lib/truncateByCharacters";

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
  return truncateByCharacters(content.trim().replace(/\s+/g, " "), 120);
}
