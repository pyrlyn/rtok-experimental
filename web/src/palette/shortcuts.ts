// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import { useEffect, useRef } from "react";
import { PAGES, type Page } from "../pages";

/** `g <key>` opens a page. First letters collide (calls/config, sessions/skills/stats/services),
 * so the keys are picked by hand; a test keeps them unique and complete. */
export const GO: Record<Page["id"], string> = {
  overview: "o",
  plugins: "p",
  calls: "c",
  sessions: "s",
  doctor: "d",
  logs: "l",
  skills: "k",
  stats: "t",
  graph: "g",
  hosts: "h",
  config: "f",
  services: "v",
  worktrees: "w",
  usage: "u",
  tasks: "b",
};

/** How long `g` waits for its second key. */
export const GO_WINDOW_MS = 1000;

export const pageForKey = (key: string) => PAGES.find((p) => GO[p.id] === key)?.id;

/** A letter typed into a field is text, never a shortcut. */
export function isTyping(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName);
}

type Keys = Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "altKey">;

/** ⌘K on macOS, Ctrl+K elsewhere; works inside fields too, as in every other app. */
export const isPaletteKey = (e: Keys) =>
  e.key.toLowerCase() === "k" && (e.metaKey || e.ctrlKey) && !e.altKey;

export const isMac = () => /Mac|iPhone|iPad/.test(navigator.platform);

export interface ShortcutHandlers {
  palette(): void;
  help(): void;
  go(page: Page["id"]): void;
}

export function useShortcuts(handlers: ShortcutHandlers) {
  // The listener stays mounted once; fresh handlers are read through the ref so a re-render
  // never drops a pending `g`.
  const latest = useRef(handlers);
  latest.current = handlers;

  useEffect(() => {
    let pending = 0;
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return;
      if (isPaletteKey(e)) {
        e.preventDefault();
        latest.current.palette();
        return;
      }
      if (e.metaKey || e.ctrlKey || e.altKey || isTyping(e.target)) return;
      if (pending > 0 && e.timeStamp - pending < GO_WINDOW_MS) {
        pending = 0;
        const page = pageForKey(e.key);
        if (page) {
          e.preventDefault();
          latest.current.go(page);
        }
        return;
      }
      if (e.key === "g") pending = e.timeStamp;
      else if (e.key === "?") {
        e.preventDefault();
        latest.current.help();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}
