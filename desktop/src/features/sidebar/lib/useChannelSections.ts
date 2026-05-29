import * as React from "react";

import {
  DEFAULT_STORE,
  readChannelSectionsStore,
  writeChannelSectionsStore,
} from "./channelSectionsStorage";

export type { ChannelSection } from "./channelSectionsStorage";

import type {
  ChannelSection,
  ChannelSectionStore,
} from "./channelSectionsStorage";

const STORAGE_KEY_PREFIX = "sprout-channel-sections.v1";

function storageKey(pubkey: string): string {
  return `${STORAGE_KEY_PREFIX}:${pubkey}`;
}

export function useChannelSections(pubkey: string | undefined): {
  sections: ChannelSection[];
  assignments: Record<string, string>;
  createSection: (name: string) => ChannelSection;
  renameSection: (sectionId: string, newName: string) => void;
  deleteSection: (sectionId: string) => void;
  moveSectionUp: (sectionId: string) => void;
  moveSectionDown: (sectionId: string) => void;
  assignChannel: (channelId: string, sectionId: string) => void;
  unassignChannel: (channelId: string) => void;
} {
  const [store, setStore] = React.useState<ChannelSectionStore>(() => {
    if (!pubkey) {
      return DEFAULT_STORE;
    }
    return readChannelSectionsStore(pubkey);
  });

  React.useEffect(() => {
    if (!pubkey) {
      setStore(DEFAULT_STORE);
      return;
    }
    setStore(readChannelSectionsStore(pubkey));
  }, [pubkey]);

  React.useEffect(() => {
    if (!pubkey) {
      return;
    }
    const key = storageKey(pubkey);
    const handler = (e: StorageEvent) => {
      if (e.key !== key) {
        return;
      }
      setStore(readChannelSectionsStore(pubkey));
    };
    window.addEventListener("storage", handler);
    return () => {
      window.removeEventListener("storage", handler);
    };
  }, [pubkey]);

  const sections = React.useMemo<ChannelSection[]>(
    () => store.sections.slice().sort((a, b) => a.order - b.order),
    [store.sections],
  );

  const createSection = React.useCallback(
    (name: string): ChannelSection => {
      let created!: ChannelSection;
      if (!pubkey) {
        created = { id: crypto.randomUUID(), name, order: 0 };
        return created;
      }
      setStore((prev) => {
        const maxOrder =
          prev.sections.length > 0
            ? Math.max(...prev.sections.map((s) => s.order))
            : -1;
        const section: ChannelSection = {
          id: crypto.randomUUID(),
          name,
          order: maxOrder + 1,
        };
        created = section;
        const next: ChannelSectionStore = {
          ...prev,
          sections: [...prev.sections, section],
        };
        if (!writeChannelSectionsStore(pubkey, next)) {
          return prev;
        }
        return next;
      });
      return created;
    },
    [pubkey],
  );

  const renameSection = React.useCallback(
    (sectionId: string, newName: string) => {
      if (!pubkey) {
        return;
      }
      setStore((prev) => {
        const next: ChannelSectionStore = {
          ...prev,
          sections: prev.sections.map((s) =>
            s.id === sectionId ? { ...s, name: newName } : s,
          ),
        };
        if (!writeChannelSectionsStore(pubkey, next)) {
          return prev;
        }
        return next;
      });
    },
    [pubkey],
  );

  const deleteSection = React.useCallback(
    (sectionId: string) => {
      if (!pubkey) {
        return;
      }
      setStore((prev) => {
        const assignments = { ...prev.assignments };
        for (const channelId of Object.keys(assignments)) {
          if (assignments[channelId] === sectionId) {
            delete assignments[channelId];
          }
        }
        const next: ChannelSectionStore = {
          ...prev,
          sections: prev.sections.filter((s) => s.id !== sectionId),
          assignments,
        };
        if (!writeChannelSectionsStore(pubkey, next)) {
          return prev;
        }
        return next;
      });
    },
    [pubkey],
  );

  const moveSectionUp = React.useCallback(
    (sectionId: string) => {
      if (!pubkey) {
        return;
      }
      setStore((prev) => {
        const target = prev.sections.find((s) => s.id === sectionId);
        if (!target) {
          return prev;
        }
        const sorted = prev.sections.slice().sort((a, b) => a.order - b.order);
        const idx = sorted.findIndex((s) => s.id === sectionId);
        if (idx <= 0) {
          return prev;
        }
        const neighbor = sorted[idx - 1];
        const sections = prev.sections.map((s) => {
          if (s.id === target.id) {
            return { ...s, order: neighbor.order };
          }
          if (s.id === neighbor.id) {
            return { ...s, order: target.order };
          }
          return s;
        });
        const next: ChannelSectionStore = { ...prev, sections };
        if (!writeChannelSectionsStore(pubkey, next)) {
          return prev;
        }
        return next;
      });
    },
    [pubkey],
  );

  const moveSectionDown = React.useCallback(
    (sectionId: string) => {
      if (!pubkey) {
        return;
      }
      setStore((prev) => {
        const target = prev.sections.find((s) => s.id === sectionId);
        if (!target) {
          return prev;
        }
        const sorted = prev.sections.slice().sort((a, b) => a.order - b.order);
        const idx = sorted.findIndex((s) => s.id === sectionId);
        if (idx < 0 || idx >= sorted.length - 1) {
          return prev;
        }
        const neighbor = sorted[idx + 1];
        const sections = prev.sections.map((s) => {
          if (s.id === target.id) {
            return { ...s, order: neighbor.order };
          }
          if (s.id === neighbor.id) {
            return { ...s, order: target.order };
          }
          return s;
        });
        const next: ChannelSectionStore = { ...prev, sections };
        if (!writeChannelSectionsStore(pubkey, next)) {
          return prev;
        }
        return next;
      });
    },
    [pubkey],
  );

  const assignChannel = React.useCallback(
    (channelId: string, sectionId: string) => {
      if (!pubkey) {
        return;
      }
      setStore((prev) => {
        const next: ChannelSectionStore = {
          ...prev,
          assignments: { ...prev.assignments, [channelId]: sectionId },
        };
        if (!writeChannelSectionsStore(pubkey, next)) {
          return prev;
        }
        return next;
      });
    },
    [pubkey],
  );

  const unassignChannel = React.useCallback(
    (channelId: string) => {
      if (!pubkey) {
        return;
      }
      setStore((prev) => {
        const assignments = { ...prev.assignments };
        delete assignments[channelId];
        const next: ChannelSectionStore = { ...prev, assignments };
        if (!writeChannelSectionsStore(pubkey, next)) {
          return prev;
        }
        return next;
      });
    },
    [pubkey],
  );

  return {
    sections,
    assignments: store.assignments,
    createSection,
    renameSection,
    deleteSection,
    moveSectionUp,
    moveSectionDown,
    assignChannel,
    unassignChannel,
  };
}
