import { create } from 'zustand'

/**
 * Persisted favorites + MRU for the Phase 3 plug-in picker flyout.
 *
 * Pinned ids appear first in the flyout (manual user pin). Recent ids
 * follow, capped at 10 and ordered most-recent-first. The flyout shows
 * up to 7 entries (pinned + recent, deduped) so we trim here on read.
 *
 * Mirrors the localStorage pattern used by `mixerSettingsStore` — direct
 * synchronous read on first import, synchronous write on every mutation.
 * Kept tiny on purpose: no Zustand persist middleware, no SSR concerns,
 * no async hydration race.
 */

const STORAGE_KEY = 'hardwave.daw.pluginFavorites'
const RECENT_CAP = 10
const FLYOUT_CAP = 7

interface StoredFavorites {
  pinned?: string[]
  recent?: string[]
}

interface FavoritesState {
  pinned: string[]
  recent: string[]
  pin: (id: string) => void
  unpin: (id: string) => void
  togglePin: (id: string) => void
  /** Bump `id` to the front of `recent`. Call after a successful pick. */
  markUsed: (id: string) => void
  /** Up to FLYOUT_CAP ids: pinned first, then recent, deduped. */
  getFlyoutList: () => string[]
}

function hydrate(): StoredFavorites {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (raw) return JSON.parse(raw) as StoredFavorites
  } catch {
    /* ignore — first run or corrupt blob */
  }
  return {}
}

function persist(next: StoredFavorites) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(next))
  } catch {
    /* ignore — quota / private mode */
  }
}

const initial = hydrate()

export const usePluginFavoritesStore = create<FavoritesState>((set, get) => ({
  pinned: initial.pinned ?? [],
  recent: initial.recent ?? [],

  pin: (id) => {
    const { pinned, recent } = get()
    if (pinned.includes(id)) return
    const next = { pinned: [...pinned, id], recent }
    persist(next)
    set(next)
  },

  unpin: (id) => {
    const { pinned, recent } = get()
    if (!pinned.includes(id)) return
    const next = { pinned: pinned.filter((x) => x !== id), recent }
    persist(next)
    set(next)
  },

  togglePin: (id) => {
    const { pinned } = get()
    if (pinned.includes(id)) get().unpin(id)
    else get().pin(id)
  },

  markUsed: (id) => {
    const { pinned, recent } = get()
    // Filter then prepend — bumps existing recent entries to the front.
    const trimmed = recent.filter((x) => x !== id)
    const nextRecent = [id, ...trimmed].slice(0, RECENT_CAP)
    const next = { pinned, recent: nextRecent }
    persist(next)
    set(next)
  },

  getFlyoutList: () => {
    const { pinned, recent } = get()
    const seen = new Set<string>()
    const out: string[] = []
    // Pinned first (preserve user order)
    for (const id of pinned) {
      if (!seen.has(id)) {
        seen.add(id)
        out.push(id)
        if (out.length >= FLYOUT_CAP) return out
      }
    }
    // Then recent
    for (const id of recent) {
      if (!seen.has(id)) {
        seen.add(id)
        out.push(id)
        if (out.length >= FLYOUT_CAP) return out
      }
    }
    return out
  },
}))
