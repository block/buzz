// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT } from "@/shared/i18n/uiTranslation";
export type Channel = {
  name: string;
  unread?: number;
  active?: boolean;
  variant: "public" | "private" | "direct";
};

export type ChannelSection = {
  title: string;
  items: Channel[];
};

export const sidebarSections: ChannelSection[] = [
  {
    get title() {
      return uiT("Pinned");
    },
    items: [
      {
        name: "general",
        unread: 1,
        variant: "public",
      },
      {
        name: "product-launch",
        unread: 3,
        active: true,
        variant: "public",
      },
    ],
  },
  {
    get title() {
      return uiT("Channels");
    },
    items: [
      {
        name: "engineering",
        variant: "public",
      },
      {
        name: "planning",
        variant: "private",
      },
    ],
  },
  {
    get title() {
      return uiT("Direct Messages");
    },
    items: [
      {
        name: "Avery",
        variant: "direct",
      },
      {
        name: "Noa",
        variant: "direct",
      },
    ],
  },
];
