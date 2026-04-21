import { create } from 'zustand'

export type NotificationLevel = 'info' | 'warning' | 'error'

export interface Notification {
  id: string
  level: NotificationLevel
  message: string
  detail?: string
  createdAt: number
  sticky: boolean
}

interface NotificationState {
  notifications: Notification[]
  push: (level: NotificationLevel, message: string, opts?: { detail?: string; sticky?: boolean }) => string
  dismiss: (id: string) => void
  clear: () => void
}

const AUTO_DISMISS_MS = 6000
const autoTimers = new Map<string, ReturnType<typeof setTimeout>>()

function newId() {
  return 'n_' + Date.now().toString(36) + Math.random().toString(36).slice(2, 8)
}

export const useNotificationStore = create<NotificationState>((set, get) => ({
  notifications: [],

  push: (level, message, opts) => {
    const id = newId()
    const sticky = opts?.sticky ?? level === 'error'
    const n: Notification = {
      id, level, message,
      detail: opts?.detail,
      createdAt: Date.now(),
      sticky,
    }
    set(s => ({ notifications: [...s.notifications, n] }))
    if (!sticky) {
      const t = setTimeout(() => get().dismiss(id), AUTO_DISMISS_MS)
      autoTimers.set(id, t)
    }
    return id
  },

  dismiss: (id) => {
    const t = autoTimers.get(id)
    if (t) { clearTimeout(t); autoTimers.delete(id) }
    set(s => ({ notifications: s.notifications.filter(n => n.id !== id) }))
  },

  clear: () => {
    for (const t of autoTimers.values()) clearTimeout(t)
    autoTimers.clear()
    set({ notifications: [] })
  },
}))
