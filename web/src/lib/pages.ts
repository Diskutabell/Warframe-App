// The pages, in the sidebar's order (lib/nav.ts), each with its component.

import type { Component } from "svelte";
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
import { NAV, navKey, type NavItem } from "./nav";

// Every page takes the summary; most ignore it.
const COMPONENTS: Record<string, Component<any>> = {
  home: Home, mastery: Mastery, primes: Primes, relics: Relics, fissures: Fissures, world: World, standing: Standing,
  foundry: Foundry, market: Market, farming: Farming, inventory: Inventory, settings: Settings,
};

export type PageDef = NavItem & { component: Component<any> };

export const PAGES: PageDef[] = NAV.map((p) => ({ ...p, component: COMPONENTS[p.id] }));

/** The key that opens a page: 1-9 for the first nine, 0 for Settings. */
export const pageKey = (p: PageDef) => navKey(p.id);
