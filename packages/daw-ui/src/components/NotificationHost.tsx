import { hw } from '../theme'
import { useNotificationStore, type NotificationLevel } from '../stores/notificationStore'

const COLORS: Record<NotificationLevel, { fg: string; border: string; bg: string }> = {
  info: { fg: hw.accent, border: 'rgba(124,201,255,0.45)', bg: 'rgba(124,201,255,0.10)' },
  warning: { fg: hw.yellow, border: 'rgba(245,158,11,0.55)', bg: 'rgba(245,158,11,0.10)' },
  error: { fg: hw.red, border: 'rgba(220,38,38,0.55)', bg: 'rgba(220,38,38,0.12)' },
}

const ICONS: Record<NotificationLevel, string> = {
  info: 'ℹ',
  warning: '⚠',
  error: '✕',
}

export function NotificationHost() {
  const notifications = useNotificationStore(s => s.notifications)
  const dismiss = useNotificationStore(s => s.dismiss)
  if (notifications.length === 0) return null
  return (
    <div
      data-testid="notification-host"
      style={{
        position: 'fixed', bottom: 16, right: 16, zIndex: 12000,
        display: 'flex', flexDirection: 'column', gap: 6,
        maxWidth: 360,
        pointerEvents: 'none',
      }}
    >
      {notifications.map(n => {
        const c = COLORS[n.level]
        return (
          <div
            key={n.id}
            role="status"
            style={{
              pointerEvents: 'auto',
              padding: '8px 10px',
              display: 'flex', gap: 8, alignItems: 'flex-start',
              background: `linear-gradient(180deg, ${c.bg}, rgba(12,12,18,0.92))`,
              border: `1px solid ${c.border}`,
              borderRadius: hw.radius.md,
              boxShadow: '0 8px 28px rgba(0,0,0,0.5)',
              backdropFilter: hw.blur.md,
              fontSize: 11, color: hw.textPrimary,
            }}
          >
            <span style={{ color: c.fg, fontSize: 12, lineHeight: 1.2, flexShrink: 0 }}>
              {ICONS[n.level]}
            </span>
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontWeight: 600, color: hw.textPrimary }}>{n.message}</div>
              {n.detail && (
                <div style={{
                  marginTop: 3, fontSize: 10, color: hw.textFaint,
                  whiteSpace: 'pre-wrap', wordBreak: 'break-all',
                  fontFamily: 'ui-monospace, Menlo, monospace',
                }}>
                  {n.detail}
                </div>
              )}
            </div>
            <button
              onClick={() => dismiss(n.id)}
              title="Dismiss"
              style={{
                width: 18, height: 18, padding: 0, flexShrink: 0,
                background: 'transparent', border: 'none',
                color: hw.textFaint, fontSize: 12, cursor: 'pointer',
              }}
            >×</button>
          </div>
        )
      })}
    </div>
  )
}
