interface UpdateModalProps {
  version: string
  changelog: string
  date: string | null
  downloading: boolean
  progress: number
  downloaded: boolean
  error: string | null
  onUpdate: () => void
  onDismiss: () => void
}

export function UpdateModal({
  version,
  changelog,
  date,
  downloading,
  progress,
  downloaded,
  error,
  onUpdate,
  onDismiss,
}: UpdateModalProps) {
  return (
    <div style={{
      position: 'fixed', inset: 0, zIndex: 9999,
      display: 'flex', alignItems: 'center', justifyContent: 'center',
    }}>
      {/* Backdrop */}
      <div
        style={{
          position: 'absolute', inset: 0,
          background: 'rgba(0,0,0,0.65)',
          backdropFilter: 'blur(4px)',
        }}
        onClick={!downloading ? onDismiss : undefined}
      />

      {/* Modal */}
      <div style={{
        position: 'relative',
        width: '100%', maxWidth: 400,
        margin: '0 16px',
        background: '#1E1E22',
        border: '1px solid rgba(255,255,255,0.08)',
        borderRadius: 14,
        boxShadow: '0 25px 50px rgba(0,0,0,0.5)',
        overflow: 'hidden',
      }}>
        {/* Gradient accent line */}
        <div style={{
          height: 2,
          background: 'linear-gradient(90deg, #7B5AC0, #9B6DFF, #7B5AC0)',
        }} />

        {/* Header */}
        <div style={{
          display: 'flex', alignItems: 'flex-start', gap: 12,
          padding: '16px 16px 0',
        }}>
          {/* Icon */}
          <div style={{
            width: 36, height: 36, borderRadius: 10,
            background: 'linear-gradient(135deg, rgba(232,160,48,0.15), rgba(232,160,48,0.05))',
            border: '1px solid rgba(232,160,48,0.2)',
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            flexShrink: 0,
          }}>
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="#9B6DFF" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2" />
            </svg>
          </div>

          <div style={{ flex: 1, minWidth: 0 }}>
            <h2 style={{ fontSize: 14, fontWeight: 600, color: '#EEE', margin: 0 }}>
              Update Available
            </h2>
            <p style={{ fontSize: 12, color: '#777', marginTop: 2 }}>
              Hardwave DAW{' '}
              <span style={{ color: '#9B6DFF', fontFamily: "'Consolas', monospace", fontWeight: 600 }}>
                v{version}
              </span>
            </p>
          </div>

          {/* Close button */}
          {!downloading && (
            <button
              onClick={onDismiss}
              style={{
                width: 24, height: 24, display: 'flex',
                alignItems: 'center', justifyContent: 'center',
                borderRadius: 6, color: '#666',
              }}
              onMouseEnter={e => {
                e.currentTarget.style.background = 'rgba(255,255,255,0.06)'
                e.currentTarget.style.color = '#CCC'
              }}
              onMouseLeave={e => {
                e.currentTarget.style.background = 'transparent'
                e.currentTarget.style.color = '#666'
              }}
            >
              <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
                <line x1="2" y1="2" x2="10" y2="10" /><line x1="10" y1="2" x2="2" y2="10" />
              </svg>
            </button>
          )}
        </div>

        {/* Changelog */}
        <div style={{ padding: '12px 16px 10px' }}>
          {date && (
            <div style={{ fontSize: 9, color: '#555', fontFamily: "'Consolas', monospace", marginBottom: 6 }}>
              {new Date(date).toLocaleDateString('en-US', { year: 'numeric', month: 'long', day: 'numeric' })}
            </div>
          )}
          <div style={{
            background: 'rgba(255,255,255,0.02)',
            border: '1px solid rgba(255,255,255,0.06)',
            borderRadius: 10,
            padding: 12,
            maxHeight: 160,
            overflowY: 'auto',
          }}>
            <div style={{ fontSize: 9, fontWeight: 700, color: '#555', textTransform: 'uppercase', letterSpacing: 1, marginBottom: 6 }}>
              What's new
            </div>
            <div style={{ fontSize: 12, color: '#BBB', lineHeight: 1.5, whiteSpace: 'pre-wrap' }}>
              {formatChangelog(changelog)}
            </div>
          </div>
        </div>

        {/* Progress bar */}
        {downloading && (
          <div style={{ padding: '0 16px 6px', display: 'flex', alignItems: 'center', gap: 8 }}>
            <div style={{
              flex: 1, height: 5,
              background: 'rgba(255,255,255,0.06)',
              borderRadius: 3, overflow: 'hidden',
            }}>
              <div style={{
                height: '100%', width: `${Math.max(5, progress)}%`,
                background: 'linear-gradient(90deg, #7B5AC0, #9B6DFF)',
                borderRadius: 3,
                transition: 'width 300ms',
              }} />
            </div>
            <span style={{ fontSize: 10, color: '#666', fontFamily: "'Consolas', monospace", width: 28, textAlign: 'right' }}>
              {progress}%
            </span>
          </div>
        )}

        {/* Error */}
        {error && (
          <div style={{
            margin: '0 16px 6px',
            padding: '6px 10px',
            background: 'rgba(200,60,60,0.1)',
            border: '1px solid rgba(200,60,60,0.2)',
            borderRadius: 8,
            display: 'flex', alignItems: 'center', gap: 6,
          }}>
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="#C44" strokeWidth="2" strokeLinecap="round">
              <circle cx="12" cy="12" r="10" /><line x1="12" y1="8" x2="12" y2="12" /><line x1="12" y1="16" x2="12.01" y2="16" />
            </svg>
            <span style={{ fontSize: 11, color: '#C44' }}>{error}</span>
          </div>
        )}

        {/* Actions */}
        <div style={{
          display: 'flex', alignItems: 'center', gap: 8,
          padding: '10px 16px 16px',
        }}>
          {!downloading && !downloaded && (
            <>
              <button
                onClick={onDismiss}
                style={{
                  flex: 1, padding: '8px 0', borderRadius: 10,
                  background: 'rgba(255,255,255,0.04)',
                  border: '1px solid rgba(255,255,255,0.06)',
                  fontSize: 12, color: '#888',
                }}
                onMouseEnter={e => {
                  e.currentTarget.style.background = 'rgba(255,255,255,0.08)'
                  e.currentTarget.style.color = '#CCC'
                }}
                onMouseLeave={e => {
                  e.currentTarget.style.background = 'rgba(255,255,255,0.04)'
                  e.currentTarget.style.color = '#888'
                }}
              >
                Later
              </button>
              <button
                onClick={onUpdate}
                style={{
                  flex: 1, padding: '8px 0', borderRadius: 10,
                  background: 'linear-gradient(90deg, #7B5AC0, #9B6DFF)',
                  border: 'none',
                  fontSize: 12, fontWeight: 700, color: '#FFF',
                  display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 6,
                  boxShadow: '0 4px 12px rgba(232,160,48,0.15)',
                }}
                onMouseEnter={e => {
                  e.currentTarget.style.background = '#9B6DFF'
                }}
                onMouseLeave={e => {
                  e.currentTarget.style.background = 'linear-gradient(90deg, #7B5AC0, #9B6DFF)'
                }}
              >
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4" /><polyline points="7 10 12 15 17 10" /><line x1="12" y1="15" x2="12" y2="3" />
                </svg>
                Update now
              </button>
            </>
          )}
          {downloading && (
            <div style={{
              flex: 1, padding: '8px 0', borderRadius: 10,
              background: 'rgba(255,255,255,0.04)',
              border: '1px solid rgba(255,255,255,0.06)',
              fontSize: 12, color: '#888',
              display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 6,
            }}>
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#9B6DFF" strokeWidth="2" strokeLinecap="round" style={{ animation: 'spin 1s linear infinite' }}>
                <path d="M21 12a9 9 0 11-6.219-8.56" />
              </svg>
              Downloading update...
            </div>
          )}
          {downloaded && (
            <div style={{
              flex: 1, padding: '8px 0', borderRadius: 10,
              background: 'rgba(90,170,90,0.1)',
              border: '1px solid rgba(90,170,90,0.2)',
              fontSize: 12, color: '#5A5',
              display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 6,
            }}>
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M22 11.08V12a10 10 0 11-5.93-9.14" /><polyline points="22 4 12 14.01 9 11.01" />
              </svg>
              Restarting...
            </div>
          )}
        </div>
      </div>

      {/* Keyframe for spinner */}
      <style>{`@keyframes spin { to { transform: rotate(360deg); } }`}</style>
    </div>
  )
}

function formatChangelog(text: string): string {
  if (!text) return '\u2022 Bug fixes and improvements.'
  const lines = text
    .replace(/^#+\s*/gm, '')
    .split('\n')
    .map(l => l.replace(/^\s*[-*]\s*/, '').trim())
    .filter(Boolean)
  const items = lines.slice(0, 4)
  return items.map(l => `\u2022 ${l}`).join('\n')
}
