// The pages by name, in the sidebar's order, without their components: what the sidebar and the
// section links need. Keys 1-9 open the first nine, 0 opens Settings.

import type { IconName } from "./icons";
import type { Summary } from "../types/Summary";

export type NavItem = {
  id: string;
  title: string;
  icon: IconName;
  /** A count beside its name in the sidebar. */
  badge?: (S: Summary) => number;
  badgeTitle?: (n: number) => string;
  /** The first page of a group: the sidebar leaves space above it. */
  group?: boolean;
};

export const NAV: NavItem[] = [
  { id: "home", title: "Overview", icon: "home" },
  { id: "mastery", title: "Mastery", icon: "mastery", group: true },
  { id: "primes", title: "Prime sets", icon: "primes" },
  { id: "relics", title: "Relics", icon: "relics" },
  { id: "fissures", title: "Fissures", icon: "fissures", group: true },
  { id: "world", title: "World", icon: "world" },
  { id: "standing", title: "Standing", icon: "standing" },
  { id: "foundry", title: "Foundry", icon: "foundry", group: true },
  { id: "market", title: "Market", icon: "market" },
  {
    id: "farming", title: "Farming", icon: "tasks",
    badge: (S) => S.tasks.done, badgeTitle: (n) => `${n} farming task${n === 1 ? "" : "s"} done`,
  },
  { id: "inventory", title: "Inventory", icon: "inventory" },
  { id: "settings", title: "Settings", icon: "settings", group: true },
];

/** The key that opens a page: 1-9 for the first nine, 0 for Settings. */
export function navKey(id: string): string {
  const i = NAV.findIndex((p) => p.id === id);
  return id === "settings" ? "0" : i >= 0 && i < 9 ? String(i + 1) : "";
}

export const navTitle = (id: string) => NAV.find((p) => p.id === id)?.title ?? id;
