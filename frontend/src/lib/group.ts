import type { Item } from "./reducer";

export interface Group {
  key: string;
  label: string;
  own: boolean;
  items: Item[];
}

export const ASSISTANT_LABEL = "Assistant";

export function groupItems(items: Item[], me: string | null): Group[] {
  const groups: Group[] = [];
  for (const item of items) {
    const key = item.kind === "prompt" ? `user:${item.author}` : "assistant";
    const last = groups.at(-1);
    if (last && last.key === key) {
      last.items.push(item);
    } else {
      groups.push({
        key,
        label: item.kind === "prompt" ? item.author : ASSISTANT_LABEL,
        own: item.kind === "prompt" && item.author === me,
        items: [item],
      });
    }
  }
  return groups;
}
