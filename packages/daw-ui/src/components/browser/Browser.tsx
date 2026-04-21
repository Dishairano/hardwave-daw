import { useEffect, useMemo, useRef, useState } from 'react'
import { hw } from '../../theme'
import { usePluginStore } from '../../stores/pluginStore'
import { useTrackStore } from '../../stores/trackStore'
import { useBrowserStore, type FolderNode } from '../../stores/browserStore'
import { DetachButton } from '../FloatingWindow'

type Tab = 'plugins' | 'files' | 'project'

export function Browser() {
  const [activeTab, setActiveTab] = useState<Tab>('plugins')

  return (
    <div style={{
      flex: 1,
      width: '100%',
      minWidth: 240,
      background: 'rgba(255,255,255,0.02)',
      backdropFilter: hw.blur.sm,
      borderRight: `1px solid ${hw.border}`,
      display: 'flex',
      flexDirection: 'column',
    }}>
      <div style={{
        display: 'flex',
        background: 'rgba(255,255,255,0.01)',
        borderBottom: `1px solid ${hw.border}`,
      }}>
        {(['plugins', 'files', 'project'] as Tab[]).map(tab => (
          <button
            key={tab}
            onClick={() => setActiveTab(tab)}
            style={{
              flex: 1, padding: '5px 0',
              fontSize: 10, fontWeight: activeTab === tab ? 600 : 400,
              color: activeTab === tab ? hw.textPrimary : hw.textFaint,
              background: activeTab === tab ? 'rgba(255,255,255,0.04)' : 'transparent',
              borderBottom: activeTab === tab ? `2px solid ${hw.accent}` : '2px solid transparent',
              transition: 'all 0.15s',
            }}
          >
            {tab.charAt(0).toUpperCase() + tab.slice(1)}
          </button>
        ))}
        <div style={{ display: 'flex', alignItems: 'center', padding: '0 4px' }}>
          <DetachButton panelId="browser" />
        </div>
      </div>

      <div style={{ flex: 1, overflowY: 'auto', padding: '2px 0' }}>
        {activeTab === 'plugins' && <PluginsTab />}
        {activeTab === 'files' && <FilesTab />}
        {activeTab === 'project' && (
          <div style={{ padding: 16, textAlign: 'center', color: hw.textFaint, fontSize: 10 }}>
            Project files will<br />appear here
          </div>
        )}
      </div>
    </div>
  )
}

function PluginsTab() {
  const { plugins, scanning, scanPlugins } = usePluginStore()
  const { selectedTrackId } = useTrackStore()
  const pluginFavorites = useBrowserStore(s => s.pluginFavorites)
  const toggleFav = useBrowserStore(s => s.togglePluginFavorite)
  const [query, setQuery] = useState('')
  const [expanded, setExpanded] = useState<Record<string, boolean>>({
    Favorites: true, Instruments: true, Effects: true,
  })

  const q = query.trim().toLowerCase()
  const matches = (p: any) =>
    !q || p.name.toLowerCase().includes(q) || (p.vendor || '').toLowerCase().includes(q)
      || (p.category || '').toLowerCase().includes(q)

  const favorites = useMemo(
    () => plugins.filter(p => pluginFavorites.has(p.id) && matches(p)),
    [plugins, pluginFavorites, q],
  )
  const instruments = useMemo(
    () => plugins.filter(p => (p.category === 'Instrument' || p.category === 'Synth') && matches(p)),
    [plugins, q],
  )
  const effects = useMemo(
    () => plugins.filter(p => p.category !== 'Instrument' && p.category !== 'Synth' && matches(p)),
    [plugins, q],
  )

  return (
    <>
      <div style={{ padding: '4px 6px', display: 'flex', gap: 4 }}>
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search plugins..."
          data-testid="plugin-search"
          style={{
            flex: 1, padding: '3px 6px', fontSize: 10,
            background: 'rgba(255,255,255,0.04)', color: hw.textPrimary,
            border: `1px solid ${hw.border}`, borderRadius: hw.radius.md, outline: 'none',
          }}
        />
        <button
          onClick={scanPlugins}
          disabled={scanning}
          title="Scan for VST3 & CLAP plugins"
          style={{
            padding: '3px 8px', fontSize: 10, color: hw.textSecondary,
            background: 'rgba(255,255,255,0.04)', border: `1px solid ${hw.border}`,
            borderRadius: hw.radius.md, opacity: scanning ? 0.5 : 1,
          }}
        >
          {scanning ? '...' : 'Scan'}
        </button>
      </div>

      {plugins.length === 0 && !scanning && (
        <div style={{ padding: 16, textAlign: 'center', color: hw.textFaint, fontSize: 10 }}>
          Click Scan to find<br />VST3 & CLAP plugins
        </div>
      )}

      {plugins.length > 0 && (
        <>
          {favorites.length > 0 && (
            <TreeGroup
              label="Favorites"
              count={favorites.length}
              expanded={expanded.Favorites}
              onToggle={() => setExpanded(p => ({ ...p, Favorites: !p.Favorites }))}
            >
              {favorites.map(p => (
                <PluginItem key={p.id} plugin={p} canAdd={!!selectedTrackId}
                  isFavorite={pluginFavorites.has(p.id)} onToggleFavorite={() => toggleFav(p.id)} />
              ))}
            </TreeGroup>
          )}
          <TreeGroup
            label="Instruments"
            count={instruments.length}
            expanded={expanded.Instruments}
            onToggle={() => setExpanded(p => ({ ...p, Instruments: !p.Instruments }))}
          >
            {instruments.map(p => (
              <PluginItem key={p.id} plugin={p} canAdd={!!selectedTrackId}
                isFavorite={pluginFavorites.has(p.id)} onToggleFavorite={() => toggleFav(p.id)} />
            ))}
            {instruments.length === 0 && <EmptyRow />}
          </TreeGroup>
          <TreeGroup
            label="Effects"
            count={effects.length}
            expanded={expanded.Effects}
            onToggle={() => setExpanded(p => ({ ...p, Effects: !p.Effects }))}
          >
            {effects.map(p => (
              <PluginItem key={p.id} plugin={p} canAdd={!!selectedTrackId}
                isFavorite={pluginFavorites.has(p.id)} onToggleFavorite={() => toggleFav(p.id)} />
            ))}
            {effects.length === 0 && <EmptyRow />}
          </TreeGroup>
        </>
      )}
    </>
  )
}

function FilesTab() {
  const fileRecents = useBrowserStore(s => s.fileRecents)
  const fileFavorites = useBrowserStore(s => s.fileFavorites)
  const folders = useBrowserStore(s => s.folders)
  const fileFolderMap = useBrowserStore(s => s.fileFolderMap)
  const expandedFolders = useBrowserStore(s => s.expandedFolders)
  const pushRecent = useBrowserStore(s => s.pushFileRecent)
  const removeRecent = useBrowserStore(s => s.removeFileRecent)
  const clearRecents = useBrowserStore(s => s.clearFileRecents)
  const toggleFav = useBrowserStore(s => s.toggleFileFavorite)
  const createFolder = useBrowserStore(s => s.createFolder)
  const renameFolder = useBrowserStore(s => s.renameFolder)
  const deleteFolder = useBrowserStore(s => s.deleteFolder)
  const moveFolder = useBrowserStore(s => s.moveFolder)
  const moveFile = useBrowserStore(s => s.moveFile)
  const toggleFolderExpanded = useBrowserStore(s => s.toggleFolderExpanded)
  const { selectedTrackId, importAudioFile } = useTrackStore()
  const [query, setQuery] = useState('')
  const [previewing, setPreviewing] = useState<string | null>(null)
  const [rootHover, setRootHover] = useState(false)
  const [previewVolume, setPreviewVolume] = useState<number>(() => {
    const raw = localStorage.getItem('hardwave.daw.previewVolume')
    const v = raw ? parseFloat(raw) : 0.7
    return Number.isFinite(v) ? Math.max(0, Math.min(1, v)) : 0.7
  })
  const currentAudioRef = useRef<HTMLAudioElement | null>(null)

  useEffect(() => {
    localStorage.setItem('hardwave.daw.previewVolume', String(previewVolume))
    if (currentAudioRef.current) currentAudioRef.current.volume = previewVolume
  }, [previewVolume])

  const q = query.trim().toLowerCase()
  const filter = (p: string) => !q || p.toLowerCase().includes(q)

  const favoritePaths = useMemo(() => Array.from(fileFavorites).filter(filter), [fileFavorites, q])
  const recents = fileRecents.filter(p => !fileFavorites.has(p) && filter(p))

  const childFolders = (parentId: string | null) => folders.filter(f => f.parentId === parentId)
  const filesInFolder = (folderId: string | null) =>
    favoritePaths.filter(p => (fileFolderMap[p] ?? null) === folderId)

  const pickFile = async () => {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog')
      const selected = await open({
        multiple: false,
        filters: [{ name: 'Audio', extensions: ['wav', 'mp3', 'flac', 'aiff', 'aif', 'ogg'] }],
      })
      if (typeof selected === 'string') pushRecent(selected)
    } catch {}
  }

  const importOne = async (path: string) => {
    if (!selectedTrackId) return
    try {
      await importAudioFile(selectedTrackId, path)
      pushRecent(path)
    } catch {}
  }

  const preview = async (path: string) => {
    if (currentAudioRef.current) {
      currentAudioRef.current.pause()
      currentAudioRef.current = null
    }
    if (previewing === path) { setPreviewing(null); return }
    try {
      const { convertFileSrc } = await import('@tauri-apps/api/core')
      const audio = new Audio(convertFileSrc(path))
      audio.volume = previewVolume
      audio.play().catch(() => {})
      currentAudioRef.current = audio
      setPreviewing(path)
      audio.onended = () => {
        if (currentAudioRef.current === audio) currentAudioRef.current = null
        setPreviewing(p => p === path ? null : p)
      }
      setTimeout(() => {
        if (currentAudioRef.current === audio) {
          audio.pause()
          currentAudioRef.current = null
          setPreviewing(p => p === path ? null : p)
        }
      }, 12000)
    } catch {
      setPreviewing(null)
    }
  }

  const handleNewFolder = (parentId: string | null) => {
    const name = window.prompt(parentId ? 'Subfolder name' : 'Folder name', 'New Folder')
    if (name && name.trim()) createFolder(name, parentId)
  }

  const handleRenameFolder = (id: string, currentName: string) => {
    const name = window.prompt('Rename folder', currentName)
    if (name && name.trim()) renameFolder(id, name)
  }

  const handleDeleteFolder = (id: string, name: string) => {
    const confirmed = window.confirm(`Delete folder "${name}"? Files and subfolders move up one level.`)
    if (confirmed) deleteFolder(id)
  }

  const handleDropOnTarget = (e: React.DragEvent, targetFolderId: string | null) => {
    e.preventDefault()
    e.stopPropagation()
    const data = e.dataTransfer.getData('application/x-hw-browser')
    if (!data) return
    const [kind, ...rest] = data.split(':')
    const payload = rest.join(':')
    if (kind === 'folder') moveFolder(payload, targetFolderId)
    else if (kind === 'file') moveFile(payload, targetFolderId)
  }

  const renderFolder = (folder: FolderNode, depth: number) => {
    const expanded = expandedFolders.has(folder.id)
    const subFolders = childFolders(folder.id)
    const files = filesInFolder(folder.id)
    const count = subFolders.length + files.length
    return (
      <FolderRow
        key={folder.id}
        folder={folder}
        depth={depth}
        expanded={expanded}
        count={count}
        onToggle={() => toggleFolderExpanded(folder.id)}
        onAddSub={() => handleNewFolder(folder.id)}
        onRename={() => handleRenameFolder(folder.id, folder.name)}
        onDelete={() => handleDeleteFolder(folder.id, folder.name)}
        onDropHere={(e) => handleDropOnTarget(e, folder.id)}
      >
        {subFolders.map(sf => renderFolder(sf, depth + 1))}
        {files.map(p => (
          <FileItem
            key={p} path={p} depth={depth + 1}
            isFavorite
            isPreviewing={previewing === p}
            onToggleFavorite={() => toggleFav(p)}
            onPreview={() => preview(p)}
            onImport={() => importOne(p)}
            onRemove={() => { moveFile(p, null); toggleFav(p); removeRecent(p) }}
          />
        ))}
      </FolderRow>
    )
  }

  const rootFolders = childFolders(null)
  const rootFiles = filesInFolder(null)
  const favHasAny = rootFolders.length > 0 || rootFiles.length > 0 || favoritePaths.length > 0

  return (
    <>
      <div style={{ padding: '4px 6px', display: 'flex', gap: 4 }}>
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search files..."
          data-testid="file-search"
          style={{
            flex: 1, padding: '3px 6px', fontSize: 10,
            background: 'rgba(255,255,255,0.04)', color: hw.textPrimary,
            border: `1px solid ${hw.border}`, borderRadius: hw.radius.md, outline: 'none',
          }}
        />
        <button
          onClick={() => handleNewFolder(null)}
          title="New folder"
          style={{
            padding: '3px 8px', fontSize: 10, color: hw.textSecondary,
            background: 'rgba(255,255,255,0.04)', border: `1px solid ${hw.border}`,
            borderRadius: hw.radius.md,
          }}
        >
          +Folder
        </button>
        <button
          onClick={pickFile}
          title="Add audio file"
          style={{
            padding: '3px 8px', fontSize: 10, color: hw.textSecondary,
            background: 'rgba(255,255,255,0.04)', border: `1px solid ${hw.border}`,
            borderRadius: hw.radius.md,
          }}
        >
          Add
        </button>
      </div>

      <div style={{
        padding: '2px 8px 4px', display: 'flex', alignItems: 'center', gap: 6,
        fontSize: 9, color: hw.textFaint,
        borderBottom: `1px solid ${hw.border}`,
      }}>
        <span title="Preview volume" style={{ fontSize: 10 }}>🔊</span>
        <input
          type="range" min={0} max={1} step={0.01}
          value={previewVolume}
          onChange={(e) => setPreviewVolume(parseFloat(e.target.value))}
          title={`Preview volume ${Math.round(previewVolume * 100)}%`}
          data-testid="preview-volume"
          style={{ flex: 1, accentColor: hw.accent, height: 2 }}
        />
        <span style={{ minWidth: 24, textAlign: 'right' }}>
          {Math.round(previewVolume * 100)}%
        </span>
      </div>

      {previewing && (
        <WaveformStrip
          path={previewing}
          onStop={() => {
            if (currentAudioRef.current) { currentAudioRef.current.pause(); currentAudioRef.current = null }
            setPreviewing(null)
          }}
        />
      )}

      <div
        onDragOver={(e) => {
          const data = e.dataTransfer.types.includes('application/x-hw-browser')
          if (data) { e.preventDefault(); setRootHover(true) }
        }}
        onDragLeave={() => setRootHover(false)}
        onDrop={(e) => { setRootHover(false); handleDropOnTarget(e, null) }}
        style={{
          background: rootHover ? 'rgba(124,201,255,0.06)' : 'transparent',
          outline: rootHover ? `1px dashed ${hw.accent}` : 'none',
          transition: 'background 0.1s',
        }}
      >
        <TreeGroup label="Favorites" count={favoritePaths.length} expanded onToggle={() => {}}>
          {rootFolders.map(f => renderFolder(f, 0))}
          {rootFiles.map(p => (
            <FileItem
              key={p} path={p} depth={0}
              isFavorite
              isPreviewing={previewing === p}
              onToggleFavorite={() => toggleFav(p)}
              onPreview={() => preview(p)}
              onImport={() => importOne(p)}
              onRemove={() => { toggleFav(p); removeRecent(p) }}
            />
          ))}
          {!favHasAny && (
            <div style={{ padding: '4px 12px', color: hw.textFaint, fontSize: 9 }}>
              Star files or drop into folders.
            </div>
          )}
        </TreeGroup>
      </div>

      <TreeGroup
        label="Recent"
        count={recents.length}
        expanded
        onToggle={() => {}}
        actionLabel={fileRecents.length > 0 ? 'Clear' : undefined}
        onAction={fileRecents.length > 0 ? clearRecents : undefined}
      >
        {recents.map(p => (
          <FileItem
            key={p} path={p} depth={0}
            isFavorite={false}
            isPreviewing={previewing === p}
            onToggleFavorite={() => toggleFav(p)}
            onPreview={() => preview(p)}
            onImport={() => importOne(p)}
            onRemove={() => removeRecent(p)}
          />
        ))}
        {recents.length === 0 && !favHasAny && (
          <div style={{ padding: '8px 12px', color: hw.textFaint, fontSize: 10 }}>
            Click Add to pick audio files.
          </div>
        )}
      </TreeGroup>
    </>
  )
}

function TreeGroup({ label, count, expanded, onToggle, children, actionLabel, onAction }: {
  label: string; count: number; expanded: boolean; onToggle: () => void;
  children: React.ReactNode;
  actionLabel?: string; onAction?: () => void;
}) {
  return (
    <div>
      <div
        onClick={onToggle}
        style={{
          padding: '4px 8px',
          display: 'flex', alignItems: 'center', gap: 4,
          cursor: 'default', fontSize: 11,
          transition: 'background 0.15s',
        }}
        onMouseEnter={e => { e.currentTarget.style.background = 'rgba(255,255,255,0.06)' }}
        onMouseLeave={e => { e.currentTarget.style.background = 'transparent' }}
      >
        <span style={{
          fontSize: 8, color: hw.textMuted,
          transform: expanded ? 'rotate(90deg)' : 'none',
          display: 'inline-block', transition: 'transform 150ms',
        }}>
          {'▶'}
        </span>
        <span style={{ color: hw.textPrimary, fontWeight: 600 }}>{label}</span>
        <span style={{ color: hw.textFaint, marginLeft: 6 }}>{count}</span>
        {actionLabel && onAction && (
          <span
            onClick={(e) => { e.stopPropagation(); onAction() }}
            style={{
              marginLeft: 'auto', fontSize: 9, color: hw.textFaint,
              padding: '1px 6px', borderRadius: 4,
              background: 'rgba(255,255,255,0.03)',
            }}
          >
            {actionLabel}
          </span>
        )}
      </div>
      {expanded && <div style={{ paddingLeft: 8 }}>{children}</div>}
    </div>
  )
}

function FolderRow({ folder, depth, expanded, count, onToggle, onAddSub, onRename, onDelete, onDropHere, children }: {
  folder: FolderNode; depth: number;
  expanded: boolean; count: number;
  onToggle: () => void; onAddSub: () => void; onRename: () => void; onDelete: () => void;
  onDropHere: (e: React.DragEvent) => void;
  children: React.ReactNode;
}) {
  const [hover, setHover] = useState(false)
  const [dropHover, setDropHover] = useState(false)
  return (
    <div>
      <div
        draggable
        onDragStart={(e) => {
          e.dataTransfer.setData('application/x-hw-browser', `folder:${folder.id}`)
          e.dataTransfer.effectAllowed = 'move'
          e.stopPropagation()
        }}
        onDragOver={(e) => {
          if (e.dataTransfer.types.includes('application/x-hw-browser')) {
            e.preventDefault()
            e.stopPropagation()
            setDropHover(true)
          }
        }}
        onDragLeave={() => setDropHover(false)}
        onDrop={(e) => { setDropHover(false); onDropHere(e) }}
        onClick={onToggle}
        onDoubleClick={(e) => { e.stopPropagation(); onRename() }}
        onMouseEnter={() => setHover(true)}
        onMouseLeave={() => setHover(false)}
        style={{
          padding: `3px 6px 3px ${8 + depth * 12}px`,
          display: 'flex', alignItems: 'center', gap: 4,
          fontSize: 10, cursor: 'grab',
          background: dropHover ? 'rgba(124,201,255,0.12)' : (hover ? 'rgba(255,255,255,0.06)' : 'transparent'),
          outline: dropHover ? `1px dashed ${hw.accent}` : 'none',
          transition: 'background 0.1s',
        }}
      >
        <span style={{
          fontSize: 8, color: hw.textMuted,
          transform: expanded ? 'rotate(90deg)' : 'none',
          display: 'inline-block', transition: 'transform 150ms',
          width: 8,
        }}>▶</span>
        <span style={{ color: hw.yellow, fontSize: 10 }}>{expanded ? '📂' : '📁'}</span>
        <span style={{
          flex: 1, color: hw.textPrimary,
          overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
        }}>{folder.name}</span>
        <span style={{ color: hw.textFaint, fontSize: 9 }}>{count}</span>
        {hover && (
          <>
            <button
              onClick={(e) => { e.stopPropagation(); onAddSub() }}
              title="New subfolder"
              style={{
                width: 16, height: 16, padding: 0,
                background: 'transparent', border: 'none',
                color: hw.textFaint, fontSize: 11, cursor: 'pointer',
              }}
            >+</button>
            <button
              onClick={(e) => { e.stopPropagation(); onDelete() }}
              title="Delete folder"
              style={{
                width: 16, height: 16, padding: 0,
                background: 'transparent', border: 'none',
                color: hw.textFaint, fontSize: 10, cursor: 'pointer',
              }}
            >×</button>
          </>
        )}
      </div>
      {expanded && <div>{children}</div>}
    </div>
  )
}

function EmptyRow() {
  return (
    <div style={{ padding: '4px 16px', color: hw.textFaint, fontSize: 9 }}>
      (none)
    </div>
  )
}

function PluginItem({ plugin, canAdd, isFavorite, onToggleFavorite }: {
  plugin: any; canAdd: boolean; isFavorite: boolean; onToggleFavorite: () => void;
}) {
  return (
    <div
      style={{
        padding: '3px 8px 3px 16px',
        cursor: canAdd ? 'pointer' : 'default',
        opacity: canAdd ? 1 : 0.5,
        display: 'flex', alignItems: 'center', gap: 6,
        transition: 'background 0.15s',
      }}
      onClick={async () => {
        if (!canAdd) return
        const { selectedTrackId } = useTrackStore.getState()
        if (!selectedTrackId) return
        await usePluginStore.getState().addToTrack(selectedTrackId, plugin.id)
        useTrackStore.getState().fetchTracks()
      }}
      onMouseEnter={e => { if (canAdd) e.currentTarget.style.background = 'rgba(255,255,255,0.06)' }}
      onMouseLeave={e => { e.currentTarget.style.background = 'transparent' }}
    >
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{
          fontSize: 10, color: hw.textPrimary,
          overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
        }}>
          {plugin.name}
        </div>
        <div style={{ fontSize: 8, color: hw.textFaint }}>{plugin.vendor} · {plugin.format}</div>
      </div>
      <button
        onClick={(e) => { e.stopPropagation(); onToggleFavorite() }}
        title={isFavorite ? 'Remove from favorites' : 'Add to favorites'}
        style={{
          width: 18, height: 18,
          background: 'transparent', border: 'none',
          color: isFavorite ? hw.yellow : hw.textFaint,
          fontSize: 12, cursor: 'pointer', padding: 0,
        }}
      >
        {isFavorite ? '★' : '☆'}
      </button>
    </div>
  )
}

function FileItem({ path, depth = 0, isFavorite, isPreviewing, onToggleFavorite, onPreview, onImport, onRemove }: {
  path: string; depth?: number; isFavorite: boolean; isPreviewing: boolean;
  onToggleFavorite: () => void; onPreview: () => void;
  onImport: () => void; onRemove: () => void;
}) {
  const name = path.split(/[\\/]/).pop() || path
  const dir = path.slice(0, path.length - name.length).replace(/[\\/]+$/, '')
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number } | null>(null)

  useEffect(() => {
    if (!ctxMenu) return
    const close = (e: MouseEvent) => {
      const t = e.target as HTMLElement
      if (t.closest('[data-file-ctx-menu]')) return
      setCtxMenu(null)
    }
    window.addEventListener('mousedown', close)
    return () => window.removeEventListener('mousedown', close)
  }, [ctxMenu])

  const copyPath = async () => {
    setCtxMenu(null)
    try { await navigator.clipboard.writeText(path) } catch { /* ignore */ }
  }

  const copyFilename = async () => {
    setCtxMenu(null)
    try { await navigator.clipboard.writeText(name) } catch { /* ignore */ }
  }

  return (
    <div
      draggable={isFavorite}
      onDragStart={(e) => {
        if (!isFavorite) return
        e.dataTransfer.setData('application/x-hw-browser', `file:${path}`)
        e.dataTransfer.effectAllowed = 'move'
      }}
      onContextMenu={(e) => {
        e.preventDefault()
        setCtxMenu({ x: e.clientX, y: e.clientY })
      }}
      style={{
        padding: `3px 8px 3px ${16 + depth * 12}px`,
        display: 'flex', alignItems: 'center', gap: 4,
        transition: 'background 0.15s',
        cursor: isFavorite ? 'grab' : 'default',
        position: 'relative',
      }}
      onMouseEnter={e => { e.currentTarget.style.background = 'rgba(255,255,255,0.06)' }}
      onMouseLeave={e => { e.currentTarget.style.background = 'transparent' }}
    >
      <div style={{ flex: 1, minWidth: 0 }} onClick={onImport} title={`Import to selected track\n${path}`}>
        <div style={{
          fontSize: 10, color: hw.textPrimary,
          overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
          cursor: 'pointer',
        }}>
          {name}
        </div>
        {dir && (
          <div style={{
            fontSize: 8, color: hw.textFaint,
            overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
          }}>
            {dir}
          </div>
        )}
      </div>
      <button
        onClick={(e) => { e.stopPropagation(); onPreview() }}
        title="Preview"
        style={{
          width: 18, height: 18, padding: 0,
          background: 'transparent', border: 'none',
          color: isPreviewing ? hw.accent : hw.textFaint,
          cursor: 'pointer',
        }}
      >
        {isPreviewing ? '■' : '▶'}
      </button>
      <button
        onClick={(e) => { e.stopPropagation(); onToggleFavorite() }}
        title={isFavorite ? 'Remove from favorites' : 'Add to favorites'}
        style={{
          width: 18, height: 18, padding: 0,
          background: 'transparent', border: 'none',
          color: isFavorite ? hw.yellow : hw.textFaint,
          fontSize: 12, cursor: 'pointer',
        }}
      >
        {isFavorite ? '★' : '☆'}
      </button>
      <button
        onClick={(e) => { e.stopPropagation(); onRemove() }}
        title="Remove"
        style={{
          width: 16, height: 16, padding: 0,
          background: 'transparent', border: 'none',
          color: hw.textFaint, fontSize: 10, cursor: 'pointer',
        }}
      >
        ×
      </button>
      {ctxMenu && (
        <div
          data-file-ctx-menu
          onMouseDown={e => e.stopPropagation()}
          style={{
            position: 'fixed', left: ctxMenu.x, top: ctxMenu.y, zIndex: 10000,
            minWidth: 200, padding: 4,
            background: 'rgba(12,12,18,0.96)',
            border: `1px solid ${hw.borderLight}`,
            borderRadius: hw.radius.md,
            boxShadow: '0 8px 32px rgba(0,0,0,0.55)',
            backdropFilter: hw.blur.md,
          }}
        >
          <div style={{
            padding: '4px 8px 2px', fontSize: 8, color: hw.textFaint,
            letterSpacing: 0.5, textTransform: 'uppercase',
            overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
          }}>
            {name}
          </div>
          <FileMenuItem label={isPreviewing ? 'Stop preview' : 'Preview'} onClick={() => { setCtxMenu(null); onPreview() }} />
          <FileMenuItem label="Import to selected track" onClick={() => { setCtxMenu(null); onImport() }} />
          <div style={{ height: 1, background: hw.border, margin: '3px 0' }} />
          <FileMenuItem label={isFavorite ? 'Remove favorite' : 'Add to favorites'} onClick={() => { setCtxMenu(null); onToggleFavorite() }} />
          <FileMenuItem label="Copy full path" onClick={copyPath} />
          <FileMenuItem label="Copy filename" onClick={copyFilename} />
          <div style={{ height: 1, background: hw.border, margin: '3px 0' }} />
          <FileMenuItem label="Remove from list" danger onClick={() => { setCtxMenu(null); onRemove() }} />
        </div>
      )}
    </div>
  )
}

function FileMenuItem({ label, danger, onClick }: { label: string; danger?: boolean; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      style={{
        width: '100%', display: 'flex', alignItems: 'center',
        padding: '5px 8px', gap: 8, border: 'none',
        background: 'transparent', color: danger ? hw.red : hw.textSecondary,
        fontSize: 11, cursor: 'pointer', borderRadius: hw.radius.sm,
        textAlign: 'left',
      }}
      onMouseEnter={e => { (e.currentTarget as HTMLElement).style.background = 'rgba(255,255,255,0.05)' }}
      onMouseLeave={e => { (e.currentTarget as HTMLElement).style.background = 'transparent' }}
    >
      {label}
    </button>
  )
}

interface FileInfo {
  durationSec: number
  sampleRate: number
  channels: number
  sizeBytes: number
}

const waveformCache = new Map<string, number[]>()
const infoCache = new Map<string, FileInfo>()

function WaveformStrip({ path, onStop }: { path: string; onStop: () => void }) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null)
  const [peaks, setPeaks] = useState<number[] | null>(() => waveformCache.get(path) ?? null)
  const [info, setInfo] = useState<FileInfo | null>(() => infoCache.get(path) ?? null)
  const [err, setErr] = useState(false)
  const name = path.split(/[\\/]/).pop() || path

  useEffect(() => {
    let cancelled = false
    setErr(false)
    const cachedPeaks = waveformCache.get(path)
    const cachedInfo = infoCache.get(path)
    if (cachedPeaks && cachedInfo) { setPeaks(cachedPeaks); setInfo(cachedInfo); return }
    setPeaks(null)
    setInfo(null)
    ;(async () => {
      try {
        const { convertFileSrc } = await import('@tauri-apps/api/core')
        const url = convertFileSrc(path)
        const resp = await fetch(url)
        if (!resp.ok) throw new Error('fetch failed')
        const buf = await resp.arrayBuffer()
        const AC: typeof AudioContext = (window as unknown as { AudioContext: typeof AudioContext; webkitAudioContext?: typeof AudioContext }).AudioContext
          || (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext
        const ctx = new AC()
        const decoded = await ctx.decodeAudioData(buf.slice(0))
        const channel = decoded.getChannelData(0)
        const bars = 160
        const step = Math.max(1, Math.floor(channel.length / bars))
        const out: number[] = []
        for (let i = 0; i < bars; i++) {
          let max = 0
          const start = i * step
          const end = Math.min(channel.length, start + step)
          for (let j = start; j < end; j++) {
            const v = Math.abs(channel[j])
            if (v > max) max = v
          }
          out.push(max)
        }
        const meta: FileInfo = {
          durationSec: decoded.duration,
          sampleRate: decoded.sampleRate,
          channels: decoded.numberOfChannels,
          sizeBytes: buf.byteLength,
        }
        ctx.close().catch(() => {})
        if (!cancelled) {
          waveformCache.set(path, out)
          infoCache.set(path, meta)
          setPeaks(out)
          setInfo(meta)
        }
      } catch {
        if (!cancelled) setErr(true)
      }
    })()
    return () => { cancelled = true }
  }, [path])

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas || !peaks) return
    const parent = canvas.parentElement
    const dpr = window.devicePixelRatio || 1
    const w = (parent?.clientWidth ?? 220) - 8
    const h = 32
    canvas.width = Math.floor(w * dpr)
    canvas.height = Math.floor(h * dpr)
    canvas.style.width = `${w}px`
    canvas.style.height = `${h}px`
    const g = canvas.getContext('2d')
    if (!g) return
    g.scale(dpr, dpr)
    g.clearRect(0, 0, w, h)
    const barW = Math.max(1, w / peaks.length)
    g.fillStyle = hw.accent
    for (let i = 0; i < peaks.length; i++) {
      const amp = peaks[i]
      const bh = Math.max(1, amp * h)
      g.fillRect(i * barW, (h - bh) / 2, Math.max(1, barW - 1), bh)
    }
  }, [peaks])

  return (
    <div style={{
      padding: '4px 8px',
      borderBottom: `1px solid ${hw.border}`,
      background: 'rgba(255,255,255,0.02)',
    }}>
      <div style={{
        display: 'flex', alignItems: 'center', gap: 4,
        fontSize: 9, color: hw.textFaint, marginBottom: 2,
      }}>
        <span style={{
          flex: 1, color: hw.textSecondary,
          overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
        }}>{name}</span>
        <button
          onClick={onStop}
          title="Stop preview"
          style={{
            width: 16, height: 16, padding: 0,
            background: 'transparent', border: 'none',
            color: hw.accent, fontSize: 10, cursor: 'pointer',
          }}
        >■</button>
      </div>
      <div style={{ height: 32, display: 'flex', alignItems: 'center' }}>
        {peaks
          ? <canvas ref={canvasRef} />
          : <div style={{ fontSize: 9, color: hw.textFaint }}>
              {err ? 'waveform unavailable' : 'decoding…'}
            </div>
        }
      </div>
      {info && (
        <div style={{
          display: 'flex', gap: 8, marginTop: 2,
          fontSize: 9, color: hw.textFaint,
          fontFamily: 'ui-monospace, Menlo, monospace',
        }}>
          <span title="Duration">{formatDuration(info.durationSec)}</span>
          <span title="Sample rate">{(info.sampleRate / 1000).toFixed(1)} kHz</span>
          <span title="Channels">{info.channels === 1 ? 'mono' : info.channels === 2 ? 'stereo' : `${info.channels}ch`}</span>
          <span title="File size" style={{ marginLeft: 'auto' }}>{formatBytes(info.sizeBytes)}</span>
        </div>
      )}
    </div>
  )
}

function formatDuration(sec: number) {
  if (!Number.isFinite(sec) || sec < 0) return '—'
  const m = Math.floor(sec / 60)
  const s = sec - m * 60
  if (m === 0) return `${s.toFixed(2)}s`
  return `${m}:${String(Math.floor(s)).padStart(2, '0')}`
}

function formatBytes(b: number) {
  if (b < 1024) return `${b} B`
  if (b < 1024 * 1024) return `${(b / 1024).toFixed(1)} KB`
  return `${(b / (1024 * 1024)).toFixed(1)} MB`
}
