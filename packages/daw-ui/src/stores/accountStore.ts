import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'

/**
 * The Hardwave account this computer is signed in with, as the backend
 * reports it (src-tauri/src/commands/account.rs).
 *
 * The token never comes here: signing in sends the address and password to
 * the backend once, and what comes back is only what the window shows.
 * Nothing in this store is saved anywhere; the shared token file is the one
 * place the sign-in lives, so the plug-ins and the Suite see the same one.
 */

export type ProVia = 'subscription' | 'trial' | 'staff'

export type Plan =
  | { kind: 'pro'; via: ProVia; until: string | null }
  | { kind: 'free' }
  | { kind: 'unknown' }

export interface Account {
  name: string
  /** Mostly hidden by the backend: `dis…@example.com`. */
  email: string
  plan: Plan
}

export type AccountStatus =
  | { state: 'signedOut' }
  /** A sign-in is on this computer but the site no longer takes it. */
  | { state: 'ended' }
  /** Signed in as far as this computer knows; the site could not be asked. */
  | { state: 'unreachable' }
  | ({ state: 'signedIn' } & Account)

export type AccountPage = 'register' | 'forgot-password' | 'account'

/** A window coming back to the front checks again, but not more often than this. */
export const RECHECK_AFTER_MS = 10_000
/** A DAW left in front for hours still renews its sign-in. */
export const CHECK_EVERY_MS = 6 * 60 * 60 * 1000

interface AccountState {
  /** False on a DAW whose backend has no account commands yet (the window
   *  is updated separately from the program): then there is nothing to show. */
  available: boolean
  /** Null until the first check has answered. */
  status: AccountStatus | null
  /** The account last seen, kept while the site cannot be reached, so the
   *  top bar does not swap a signed-in person for "Sign in" when offline. */
  lastAccount: Account | null
  checking: boolean
  /** Signing in or out. */
  busy: boolean
  error: string | null
  open: boolean
  lastCheckedAt: number
  refresh: (opts?: { force?: boolean }) => Promise<void>
  signIn: (email: string, password: string) => Promise<boolean>
  signOut: () => Promise<void>
  setOpen: (open: boolean) => void
  clearError: () => void
  openPage: (page: AccountPage) => void
}

function accountOf(status: AccountStatus): Account | null {
  if (status.state !== 'signedIn') return null
  const { name, email, plan } = status
  return { name, email, plan }
}

const STATES = ['signedOut', 'ended', 'unreachable', 'signedIn']

function isStatus(value: unknown): value is AccountStatus {
  return typeof value === 'object' && value !== null
    && STATES.includes((value as { state?: string }).state ?? '')
}

/** What Tauri says when the program does not have the command. */
function isMissingCommand(e: unknown): boolean {
  return /Command account_status not (found|allowed)/.test(String(e))
}

/** Counts sign-ins and sign-outs, so a check that was already on its way
 *  when one happened does not put the old state back. */
let changes = 0

export const useAccountStore = create<AccountState>((set, get) => {
  const apply = (status: AccountStatus) => set(s => ({
    status,
    lastAccount: status.state === 'unreachable' ? s.lastAccount : accountOf(status),
  }))

  return {
    available: true,
    status: null,
    lastAccount: null,
    checking: false,
    busy: false,
    error: null,
    open: false,
    lastCheckedAt: 0,

    refresh: async ({ force = false } = {}) => {
      const s = get()
      if (s.checking || s.busy) return
      if (!force && Date.now() - s.lastCheckedAt < RECHECK_AFTER_MS) return
      const seen = changes
      set({ checking: true, lastCheckedAt: Date.now() })
      try {
        const status = await invoke<AccountStatus>('account_status')
        if (seen === changes) apply(isStatus(status) ? status : { state: 'unreachable' })
      } catch (e) {
        if (isMissingCommand(e)) set({ available: false })
        else if (seen === changes) apply({ state: 'unreachable' })
      } finally {
        set({ checking: false })
      }
    },

    signIn: async (email, password) => {
      changes++
      set({ busy: true, error: null })
      try {
        const status = await invoke<AccountStatus>('account_sign_in', { email, password })
        apply(isStatus(status) ? status : { state: 'unreachable' })
        set({ lastCheckedAt: Date.now() })
        return true
      } catch (e) {
        set({ error: String(e) })
        return false
      } finally {
        set({ busy: false })
      }
    },

    signOut: async () => {
      changes++
      set({ busy: true, error: null })
      try {
        await invoke('account_sign_out')
        apply({ state: 'signedOut' })
        set({ open: false })
      } catch (e) {
        set({ error: String(e) })
      } finally {
        set({ busy: false })
      }
    },

    setOpen: (open) => set({ open, error: null }),
    clearError: () => set({ error: null }),

    openPage: (page) => {
      invoke('account_open_page', { page }).catch(() => {})
    },
  }
})

/**
 * Check the account now, whenever the window comes back to the front (a
 * sign-in or sign-out in the Suite or a plug-in shows up then), and every
 * few hours. Returns the clean-up.
 */
export function watchAccount(): () => void {
  const refresh = () => { void useAccountStore.getState().refresh() }
  const onVisible = () => { if (document.visibilityState === 'visible') refresh() }
  void useAccountStore.getState().refresh({ force: true })
  window.addEventListener('focus', refresh)
  document.addEventListener('visibilitychange', onVisible)
  const timer = window.setInterval(
    () => { void useAccountStore.getState().refresh({ force: true }) },
    CHECK_EVERY_MS,
  )
  return () => {
    window.removeEventListener('focus', refresh)
    document.removeEventListener('visibilitychange', onVisible)
    window.clearInterval(timer)
  }
}

/** Two letters for the round badge: the first of each of the first two words. */
export function initialsOf(name: string): string {
  const words = name.split(/[\s._@…-]+/).filter(Boolean)
  const letters = words.slice(0, 2).map(w => [...w][0] ?? '')
  return letters.join('').toUpperCase() || '?'
}
