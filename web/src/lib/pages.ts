// The pages, in the sidebar's order. Keys 1-9 open the first nine, 0 opens Settings.

import type { Component } from "svelte";
import type { IconName } from "./icons";
import Farming from "../pages/Farming.svelte";
import Fissures from "../pages/Fissures.svelte";
import Foundry from "../pages/Foundry.svelte";
import Home from "../pages/Home.svelte";
import Inventory from "../pages/Inventory.svelte";
import Market from "../pages/Market.svelte";
import Mastery from "../pages/Mastery.svelte";
import Primes from "../pages/Primes.svelte";
import Relics from "../pages/Relics.svelte";
import Settings from "../pages/Settings.svelte";
import Standing from "../pages/Standing.svelte";
import World from "../pages/World.svelte";
import type { Summary } from "../types/Summary";

export type PageDef = {
  id: string;
  title: string;
  icon: IconName;
  // Every page takes the summary; most ignore it.
  component: Component<any>;
  /** A count beside its name in the sidebar. */
  badge?: (S: Summary) => number;
  badgeTitle?: (n: number) => string;
  /** The heading above it in the sidebar. */
  group?: string;
};

export const PAGES: PageDef[] = [
  { id: "home", title: "Overview", icon: "home", component: Home },
  { id: "mastery", title: "Mastery", icon: "mastery", component: Mastery, group: "Progression" },
  { id: "primes", title: "Prime sets", icon: "primes", component: Primes },
  { id: "relics", title: "Relics", icon: "relics", component: Relics },
  { id: "fissures", title: "Fissures", icon: "fissures", component: Fissures, group: "Origin System" },
  { id: "world", title: "World", icon: "world", component: World },
  { id: "standing", title: "Standing", icon: "standing", component: Standing },
  { id: "foundry", title: "Foundry", icon: "foundry", component: Foundry, group: "Orbiter" },
  { id: "market", title: "Market", icon: "market", component: Market },
  {
    id: "farming", title: "Farming", icon: "tasks", component: Farming,
    badge: (S) => S.tasks.done, badgeTitle: (n) => `${n} farming task${n === 1 ? "" : "s"} done`,
  },
  { id: "inventory", title: "Inventory", icon: "inventory", component: Inventory },
  { id: "settings", title: "Settings", icon: "settings", component: Settings },
];

/** The key that opens a page: 1-9 for the first nine, 0 for Settings. */
export const pageKey = (p: PageDef, i: number) => (p.id === "settings" ? "0" : i < 9 ? String(i + 1) : "");
