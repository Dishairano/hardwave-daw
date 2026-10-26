import { useEffect, useMemo, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { usePluginStore } from '../../stores/pluginStore'
import { useTrackStore } from '../../stores/trackStore'
import { t, tf, useLanguage } from '../../i18n'

export function PluginBrowser() {
  useLanguage()
  const {
    plugins,
    scanning,
    scanProgress,
    setScanProgress,
    scanPlugins,
    loadCachedPlugins,
    lastDiff,
    blocklist,
    loadBlocklist,
    toggleBlocked,
    customVst3Paths,
    customClapPaths,
    loadCustomPaths,
    setCustomPaths,
    cachePath,
    loadCachePath,
  } = usePluginStore()
  const { selectedTrackId } = useTrackStore()
  const [showSettings, setShowSettings] = useState(false)
  const [newVst3Path, setNewVst3Path] = useState('')
  const [newClapPath, setNewClapPath] = useState('')

  useEffect(() => {
    loadCachedPlugins()
    loadBlocklist()
    loadCustomPaths()
    loadCachePath()
  }, [loadCachedPlugins, loadBlocklist, loadCustomPaths, loadCachePath])

  useEffect(() => {
    const unlistenProgress = listen<{ count: number; current: string }>(
      'daw:pluginScanProgress',
      (e) => setScanProgress(e.payload)
    )
    const unlistenComplete = listen('daw:pluginScanComplete', () => setScanProgress(null))
    return () => {
      unlistenProgress.then((fn) => fn())
      unlistenComplete.then((fn) => fn())
    }
  }, [setScanProgress])

  const blocked = useMemo(() => new Set(blocklist), [blocklist])
  const hasDiff = lastDiff.added.length + lastDiff.removed.length > 0

  return (
    <div style={{
      background: '#0e0e10',
      borderLeft: '1px solid rgba(255,255,255,0.06)',
      display: 'flex',
      flexDirection: 'column',
      overflow: 'hidden',
    }}>
      <div style={{
        padding: '8px 12px',
        borderBottom: '1px solid rgba(255,255,255,0.04)',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'space-between',
        gap: 6,
      }}>
        <span style={{ fontSize: 10, fontWeight: 700, color: '#555', letterSpacing: 1 }}>{t('PLUGINS')}</span>
        <div style={{ display: 'flex', gap: 4 }}>
          <button
            onClick={() => setShowSettings((v) => !v)}
            title={t('Scan settings')}
            style={{
              padding: '2px 8px', fontSize: 9,
              background: showSettings ? 'rgba(255,255,255,0.08)' : 'rgba(255,255,255,0.04)',
              border: '1px solid rgba(255,255,255,0.08)',
              borderRadius: 4, color: '#888', cursor: 'pointer', fontFamily: 'inherit',
            }}
          >
            ⚙
          </button>
          <button
            onClick={scanPlugins}
            disabled={scanning}
            title={t('Re-scan plugin folders')}
            style={{
              padding: '2px 8px', fontSize: 9,
              background: 'rgba(255,255,255,0.04)', border: '1px solid rgba(255,255,255,0.08)',
              borderRadius: 4, color: '#666', cursor: 'pointer', fontFamily: 'inherit',
            }}
          >
            {scanning ? t('Scanning...') : t('Re-scan')}
          </button>
        </div>
      </div>

      {scanning && scanProgress && (
        <div style={{
          padding: '4px 12px', fontSize: 9, color: '#888',
          background: 'rgba(255,255,255,0.02)',
          borderBottom: '1px solid rgba(255,255,255,0.04)',
          whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
        }}>
          {tf('Scanning… {n} found · {current}', { n: scanProgress.count, current: scanProgress.current || '…' })}
        </div>
      )}

      {!scanning && hasDiff && (
        <div style={{
          padding: '4px 12px', fontSize: 9, color: '#888',
          background: 'rgba(255,255,255,0.02)',
          borderBottom: '1px solid rgba(255,255,255,0.04)',
        }}>
          {tf('+{added} new · -{removed} missing', { added: lastDiff.added.length, removed: lastDiff.removed.length })}
        </div>
      )}

      {showSettings && (
        <div style={{
          padding: '8px 12px', fontSize: 10, color: '#888',
          background: 'rgba(255,255,255,0.015)',
          borderBottom: '1px solid rgba(255,255,255,0.04)',
          display: 'flex', flexDirection: 'column', gap: 8,
        }}>
          <div style={{ fontSize: 9, color: '#555' }}>
            {t('Cache:')} <span style={{ color: '#777' }}>{cachePath ?? t('unavailable')}</span>
          </div>

          <div>
            <div style={{ fontSize: 9, color: '#555', marginBottom: 3 }}>{t('CUSTOM VST3 PATHS')}</div>
            {customVst3Paths.map((p) => (
              <div key={p} style={{ display: 'flex', justifyContent: 'space-between', padding: '2px 0' }}>
                <span style={{ fontSize: 10, color: '#aaa', overflow: 'hidden', textOverflow: 'ellipsis' }}>{p}</span>
                <button
                  onClick={() => setCustomPaths(customVst3Paths.filter((x) => x !== p), customClapPaths)}
                  style={{
                    fontSize: 9, background: 'transparent', border: 'none',
                    color: '#666', cursor: 'pointer', padding: '0 4px',
                  }}
                >×</button>
              </div>
            ))}
            <div style={{ display: 'flex', gap: 4, marginTop: 3 }}>
              <input
                type="text"
                value={newVst3Path}
                placeholder="/path/to/vst3"
                onChange={(e) => setNewVst3Path(e.target.value)}
                style={{
                  flex: 1, fontSize: 10, padding: '2px 6px',
                  background: '#0a0a0c', border: '1px solid rgba(255,255,255,0.08)',
                  borderRadius: 3, color: '#ccc', fontFamily: 'inherit',
                }}
              />
              <button
                onClick={() => {
                  const v = newVst3Path.trim()
                  if (!v) return
                  setCustomPaths([...customVst3Paths, v], customClapPaths)
                  setNewVst3Path('')
                }}
                style={{
                  fontSize: 9, padding: '2px 8px',
                  background: 'rgba(255,255,255,0.04)', border: '1px solid rgba(255,255,255,0.08)',
                  borderRadius: 3, color: '#888', cursor: 'pointer', fontFamily: 'inherit',
                }}
              >{t('Add')}</button>
            </div>
          </div>

          <div>
            <div style={{ fontSize: 9, color: '#555', marginBottom: 3 }}>{t('CUSTOM CLAP PATHS')}</div>
            {customClapPaths.map((p) => (
              <div key={p} style={{ display: 'flex', justifyContent: 'space-between', padding: '2px 0' }}>
                <span style={{ fontSize: 10, color: '#aaa', overflow: 'hidden', textOverflow: 'ellipsis' }}>{p}</span>
                <button
                  onClick={() => setCustomPaths(customVst3Paths, customClapPaths.filter((x) => x !== p))}
                  style={{
                    fontSize: 9, background: 'transparent', border: 'none',
                    color: '#666', cursor: 'pointer', padding: '0 4px',
                  }}
                >×</button>
              </div>
            ))}
            <div style={{ display: 'flex', gap: 4, marginTop: 3 }}>
              <input
                type="text"
                value={newClapPath}
                placeholder="/path/to/clap"
                onChange={(e) => setNewClapPath(e.target.value)}
                style={{
                  flex: 1, fontSize: 10, padding: '2px 6px',
                  background: '#0a0a0c', border: '1px solid rgba(255,255,255,0.08)',
                  borderRadius: 3, color: '#ccc', fontFamily: 'inherit',
                }}
              />
              <button
                onClick={() => {
                  const v = newClapPath.trim()
                  if (!v) return
                  setCustomPaths(customVst3Paths, [...customClapPaths, v])
                  setNewClapPath('')
                }}
                style={{
                  fontSize: 9, padding: '2px 8px',
                  background: 'rgba(255,255,255,0.04)', border: '1px solid rgba(255,255,255,0.08)',
                  borderRadius: 3, color: '#888', cursor: 'pointer', fontFamily: 'inherit',
                }}
              >{t('Add')}</button>
            </div>
          </div>

          <div style={{ fontSize: 9, color: '#444' }}>
            {t('New custom paths apply on the next Re-scan.')}
          </div>
        </div>
      )}

      <div style={{ flex: 1, overflowY: 'auto', padding: '4px 0' }}>
        {plugins.length === 0 && (
          <div style={{ padding: 16, textAlign: 'center', color: '#222', fontSize: 10, whiteSpace: 'pre-line' }}>
            {t('Click "Re-scan" to find\nVST3 & CLAP plugins')}
          </div>
        )}

        {plugins.map((plugin) => {
          const isBlocked = blocked.has(plugin.id)
          return (
            <div
              key={plugin.id}
              draggable={!isBlocked}
              onDragStart={(e) => {
                if (isBlocked) return
                e.dataTransfer.setData('application/x-hardwave-plugin', plugin.id)
                e.dataTransfer.effectAllowed = 'copy'
              }}
              style={{
                padding: '5px 12px',
                display: 'flex', alignItems: 'center', justifyContent: 'space-between',
                cursor: selectedTrackId && !isBlocked ? 'pointer' : 'default',
                opacity: isBlocked ? 0.35 : (selectedTrackId ? 1 : 0.5),
              }}
              onClick={async () => {
                if (!selectedTrackId || isBlocked) return
                const { addToTrack } = usePluginStore.getState()
                await addToTrack(selectedTrackId, plugin.id)
                useTrackStore.getState().fetchTracks()
              }}
              onMouseEnter={(e) => {
                if (selectedTrackId && !isBlocked) (e.currentTarget as HTMLElement).style.background = 'rgba(255,255,255,0.03)'
              }}
              onMouseLeave={(e) => {
                (e.currentTarget as HTMLElement).style.background = 'transparent'
              }}
            >
              <div style={{ overflow: 'hidden' }}>
                <div style={{ fontSize: 11, color: isBlocked ? '#555' : '#ccc', textDecoration: isBlocked ? 'line-through' : 'none' }}>
                  {plugin.name}
                </div>
                <div style={{ fontSize: 9, color: '#444' }}>
                  {plugin.vendor} — {plugin.format}
                </div>
              </div>
              <button
                onClick={(e) => { e.stopPropagation(); toggleBlocked(plugin.id) }}
                title={isBlocked ? t('Unblock plugin') : t('Block plugin')}
                style={{
                  fontSize: 9, padding: '1px 6px', marginLeft: 6,
                  background: isBlocked ? 'rgba(255, 80, 80, 0.12)' : 'rgba(255,255,255,0.04)',
                  border: '1px solid rgba(255,255,255,0.08)',
                  borderRadius: 3, color: isBlocked ? '#c55' : '#666',
                  cursor: 'pointer', fontFamily: 'inherit',
                }}
              >
                {isBlocked ? t('blocked') : t('block')}
              </button>
            </div>
          )
        })}
      </div>

      {selectedTrackId && plugins.length > 0 && (
        <div style={{ padding: '6px 12px', borderTop: '1px solid rgba(255,255,255,0.04)', fontSize: 9, color: '#333' }}>
          {t('Click a plugin to add to selected track')}
        </div>
      )}
    </div>
  )
}
