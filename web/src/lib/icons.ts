// Simple line icons drawn for this app (24x24, stroked).
export const PATHS = {
  home: "M3 11 12 4l9 7v9a1 1 0 0 1-1 1h-5v-6H9v6H4a1 1 0 0 1-1-1z",
  // A rank medal: the relic glyph below is a hexagon, so this one is not.
  mastery: "M12 3.5a5.5 5.5 0 1 0 0 11 5.5 5.5 0 0 0 0-11z M8.6 13.4 7 21l5-2.6 5 2.6-1.6-7.6 M12 6.8l.9 1.8 2 .3-1.45 1.4.35 2-1.8-.95-1.8.95.35-2L9.1 8.9l2-.3z",
  primes: "M4 7h16M4 12h16M4 17h10 M18 15l2 2-2 2",
  relics: "M12 2 20 7v10l-8 5-8-5V7z M12 7l4 2.5v5L12 17l-4-2.5v-5z",
  fissures: "M13 2 4 14h7l-1 8 9-12h-7z",
  world: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18z M3 12h18 M12 3c2.5 2.6 3.8 5.6 3.8 9s-1.3 6.4-3.8 9c-2.5-2.6-3.8-5.6-3.8-9S9.5 5.6 12 3z",
  foundry: "M4 20h16 M6 20V10l4-3v4l4-3v4l4-3v11 M14 4h4v4",
  market: "M4 8h16l-1.5 11a2 2 0 0 1-2 1.7h-9A2 2 0 0 1 5.5 19z M9 8V6a3 3 0 0 1 6 0v2",
  settings: "M12 8.5a3.5 3.5 0 1 0 0 7 3.5 3.5 0 0 0 0-7z M19.4 13.5l1.6 1.2-2 3.4-1.9-.7a7 7 0 0 1-2 1.1L14.8 21h-4l-.3-2a7 7 0 0 1-2-1.1l-1.9.7-2-3.4 1.6-1.2a7 7 0 0 1 0-2.4L4.6 10.4l2-3.4 1.9.7a7 7 0 0 1 2-1.1L10.8 4h4l.3 2.6a7 7 0 0 1 2 1.1l1.9-.7 2 3.4-1.6 1.2a7 7 0 0 1 0 2.4z",
  search: "M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14z M20 20l-4-4",
  bell: "M6 16V11a6 6 0 0 1 12 0v5l1.5 2h-15z M10 20a2 2 0 0 0 4 0",
  x: "M6 6l12 12M18 6 6 18",
  back: "M15 5l-7 7 7 7",
  next: "M9 5l7 7-7 7",
  star: "M12 3.5l2.6 5.4 5.9.8-4.3 4.1 1 5.8L12 16.9l-5.2 2.7 1-5.8L3.5 9.7l5.9-.8z",
  link: "M14 4h6v6 M20 4l-9 9 M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5",
  clock: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18z M12 7v5l3 2",
  check: "M5 12l4 4 10-10",
  plus: "M12 5v14M5 12h14",
  minus: "M5 12h14",
  refresh: "M20 11a8 8 0 1 0-2.3 5.7 M20 4v7h-7",
  standing: "M12 3l7 3v5c0 4.6-3 8.2-7 10-4-1.8-7-5.4-7-10V6z M8.5 12.5l2.4 2.4 4.6-4.8",
  down: "M6 9l6 6 6-6",
  inventory: "M4 8l8-4 8 4v9l-8 4-8-4z M4 8l8 4 8-4 M12 12v9",
  tasks: "M9 3.5h6v3H9z M9 5H6a1 1 0 0 0-1 1v14a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1V6a1 1 0 0 0-1-1h-3 M8.5 13.5l2.5 2.5 4.5-5",
  tray: "M4 15v3a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-3 M12 4v10 M8 10l4 4 4-4",
  power: "M12 3v8 M6.8 6.8a8 8 0 1 0 10.4 0",
} as const;

export type IconName = keyof typeof PATHS;
