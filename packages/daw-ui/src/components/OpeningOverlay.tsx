import { useProjectStore } from '../stores/projectStore'

const STAGES: Record<string, string> = {
  file: 'Reading the file',
  audio: 'Loading the audio',
  plugins: 'Loading the plug-ins',
  done: 'Almost there',
}

/**
 * Says what an opening song is doing. Opening used to run on the window's
 * own thread, which froze the window ("Not Responding") with no word of
 * what was happening. It runs in the background now, so the window can
 * show this, and it covers the app so nothing is edited mid-open.
 */
export function OpeningOverlay() {
  const opening = useProjectStore((s) => s.opening)
  if (!opening) return null
  const stage = STAGES[opening.stage] ?? 'Opening'
  const count = opening.stage === 'audio' && opening.total > 0 ? ` ${opening.done} of ${opening.total}` : ''
  const share = opening.total > 0 ? opening.done / opening.total : 0
  return (
    <div
      role="status"
      aria-live="polite"
      style={{
        position: 'fixed', inset: 0, zIndex: 9000, display: 'flex',
        alignItems: 'center', justifyContent: 'center',
        background: 'rgba(4, 4, 6, 0.55)',
      }}
    >
      <div
        style={{
          minWidth: 280, maxWidth: 'calc(100vw - 32px)', padding: '16px 18px',
          background: '#0c0c11', border: '1px solid rgba(255,255,255,0.08)',
          borderRadius: 8, color: '#e5e5ea', fontSize: 12,
        }}
      >
        <div style={{ fontWeight: 600, marginBottom: 6 }}>Opening {opening.name}</div>
        <div style={{ color: '#9a9aa6', marginBottom: 10 }}>{stage}{count}</div>
        <div style={{ height: 3, background: 'rgba(255,255,255,0.06)', borderRadius: 2 }}>
          <div
            style={{
              height: 3, width: `${Math.round(share * 100)}%`, background: '#DC2626',
              borderRadius: 2, transition: 'width 120ms linear',
            }}
          />
        </div>
      </div>
    </div>
  )
}
