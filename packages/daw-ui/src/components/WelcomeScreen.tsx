import { useEffect, useState } from 'react'
import { hw } from '../theme'

interface Props {
  recentProjects: string[]
  onOpenRecent: (path: string) => void
  onNewProject: () => void
  onOpenProject: () => void
  onDismiss: () => void
}

export function WelcomeScreen({
  recentProjects,
  onOpenRecent,
  onNewProject,
  onOpenProject,
  onDismiss,
}: Props) {
  const [mounted, setMounted] = useState(false)
  useEffect(() => { setMounted(true) }, [])

  return (
    <div style={{
      position: 'fixed',
      inset: 0,
      zIndex: 5000,
      background: `radial-gradient(circle at 50% 30%, rgba(220,38,38,0.08) 0%, ${hw.bg} 60%)`,
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'center',
      opacity: mounted ? 1 : 0,
      transition: 'opacity 0.25s',
    }}>
      <div style={{
        width: 720,
        maxWidth: '90vw',
        background: hw.bgPanel,
        backdropFilter: hw.blur.md,
        border: `1px solid ${hw.border}`,
        borderRadius: hw.radius.lg,
        padding: 32,
        boxShadow: hw.glowRed,
      }}>
        <div style={{
          fontSize: 28,
          fontWeight: 700,
          color: hw.textBright,
          marginBottom: 4,
          letterSpacing: -0.5,
        }}>
          Hardwave DAW
        </div>
        <div style={{ fontSize: 13, color: hw.textMuted, marginBottom: 28 }}>
          Start a new project or open a recent one.
        </div>

        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 28 }}>
          <div>
            <div style={{ fontSize: 11, color: hw.textFaint, textTransform: 'uppercase', letterSpacing: 0.8, marginBottom: 10 }}>
              Get started
            </div>
            <button
              onClick={() => { onNewProject(); onDismiss() }}
              style={actionBtn(true)}
            >
              New project
            </button>
            <button
              onClick={() => { onOpenProject(); onDismiss() }}
              style={actionBtn(false)}
            >
              Open project…
            </button>
            <button
              onClick={onDismiss}
              style={{ ...actionBtn(false), marginTop: 18 }}
            >
              Close this screen
            </button>
          </div>

          <div>
            <div style={{ fontSize: 11, color: hw.textFaint, textTransform: 'uppercase', letterSpacing: 0.8, marginBottom: 10 }}>
              Recent projects
            </div>
            {recentProjects.length === 0 ? (
              <div style={{ fontSize: 12, color: hw.textMuted, padding: '16px 0' }}>
                No recent projects yet.
              </div>
            ) : (
              <div style={{ maxHeight: 260, overflowY: 'auto' }}>
                {recentProjects.slice(0, 8).map(p => {
                  const name = p.split(/[\\/]/).pop()?.replace('.hwp', '') || p
                  return (
                    <button
                      key={p}
                      onClick={() => { onOpenRecent(p); onDismiss() }}
                      style={recentBtn}
                      title={p}
                    >
                      <div style={{ fontSize: 13, color: hw.textPrimary, fontWeight: 500 }}>{name}</div>
                      <div style={{ fontSize: 10, color: hw.textMuted, marginTop: 2, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                        {p}
                      </div>
                    </button>
                  )
                })}
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  )
}

function actionBtn(primary: boolean): React.CSSProperties {
  return {
    display: 'block',
    width: '100%',
    padding: '10px 14px',
    marginBottom: 8,
    textAlign: 'left',
    fontSize: 13,
    fontWeight: 500,
    color: primary ? hw.textBright : hw.textPrimary,
    background: primary ? hw.accent : hw.bgElevated,
    border: `1px solid ${primary ? hw.accent : hw.border}`,
    borderRadius: hw.radius.md,
    cursor: 'pointer',
  }
}

const recentBtn: React.CSSProperties = {
  display: 'block',
  width: '100%',
  padding: '8px 10px',
  marginBottom: 4,
  textAlign: 'left',
  background: 'transparent',
  border: `1px solid transparent`,
  borderRadius: hw.radius.sm,
  cursor: 'pointer',
}
