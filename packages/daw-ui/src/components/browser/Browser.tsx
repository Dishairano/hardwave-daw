import { useEffect, useMemo, useRef, useState } from 'react'
import { hw } from '../../theme'
import { usePluginStore } from '../../stores/pluginStore'
import { useTrackStore } from '../../stores/trackStore'
import { useBrowserStore, type FolderNode } from '../../stores/browserStore'
import { useSampleEditorStore } from '../../stores/sampleEditorStore'
import { useBeatSlicerStore } from '../../stores/beatSlicerStore'
import { DetachButton } from '../FloatingWindow'

type Tab = 'plugins' | 'files' | 'project'

export function Browser() {
  const [activeTab, setActiveTab] = useState<Tab>('plugins')
  // Item count shown in the header — switches with the active tab so the
  // user always sees how many items are available in the current scope.
  const pluginCount = usePluginStore(s => s.plugins.length)
  const fileFavCount = useBrowserStore(s => s.fileFavorites.size)
  const fileRecentCount = useBrowserStore(s => s.fileRecents.length)
  const folderCount = useBrowserStore(s => s.folders.length)
  const headerCount =
    activeTab === 'plugins' ? pluginCount :
    activeTab === 'files'   ? (fileFavCount + fileRecentCount + folderCount) :
    0

  return (
    // Root is intentionally unstyled — the parent .fl-browser (mockup class
    // applied by HwApp) provides width / background / border / flex layout.
    // This avoids dual chrome that fights with the mockup look.
    <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minHeight: 0 }}>
      <div className="fl-browser-head">
        <span className="disclosure" aria-hidden="true">▾</span>
        <span style={{ flex: 1 }}>Browser</span>
        {headerCount > 0 && <span className="count">{headerCount}</span>}
        <DetachButton panelId="browser" />
      </div>

      {/*
        Icon-only tab strip per mockup. We keep our 3 content semantics
        (plugins / files / project) but use SVG icons matching the
        mockup's category icons instead of text labels. Hover tooltips
        give the human-readable name.
      */}
      <div className="fl-browser-tabs">
        <button
          type="button"
          onClick={() => setActiveTab('plugins')}
          title="Plugins"
          className={`fl-browser-tab${activeTab === 'plugins' ? ' on' : ''}`}
        >
          <svg width="12" height="12" viewBox="0 0 16 16" fill="none">
            <circle cx="5" cy="12" r="2" fill="currentColor"/>
            <circle cx="11" cy="10" r="2" fill="currentColor"/>
            <path d="M7 12V3.5l6-1.5v8" fill="none" stroke="currentColor" strokeWidth="1.4"/>
          </svg>
        </button>
        <button
          type="button"
          onClick={() => setActiveTab('files')}
          title="Files"
          className={`fl-browser-tab${activeTab === 'files' ? ' on' : ''}`}
        >
          <svg width="12" height="12" viewBox="0 0 16 16" fill="none">
            <path d="M2 4.5h4l1 1h7v8H2z" fill="none" stroke="currentColor" strokeWidth="1.2" strokeLinejoin="round"/>
          </svg>
        </button>
        <button
          type="button"
          onClick={() => setActiveTab('project')}
          title="Project"
          className={`fl-browser-tab${activeTab === 'project' ? ' on' : ''}`}
        >
          <svg width="12" height="12" viewBox="0 0 16 16" fill="none">
            <path d="M3 4.5h10M3 8h10M3 11.5h10" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round"/>
          </svg>
        </button>
      </div>

      <div className="fl-browser-tree">
        {activeTab === 'plugins' && <PluginsTab />}
        {activeTab === 'files' && <FilesTab />}
        {activeTab === 'project' && (
          <div style={{
            padding: 18, textAlign: 'center',
            fontFamily: hw.font.mono, fontSize: 10,
            color: hw.textFaint, letterSpacing: hw.tracking.wide,
            textTransform: 'uppercase',
          }}>
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
  const fileTags = useBrowserStore(s => s.fileTags)
  const addFileTag = useBrowserStore(s => s.addFileTag)
  const removeFileTag = useBrowserStore(s => s.removeFileTag)
  const clearFileTags = useBrowserStore(s => s.clearFileTags)
  const { selectedTrackId, importAudioFile } = useTrackStore()
  const [query, setQuery] = useState('')
  const [previewing, setPreviewing] = useState<string | null>(null)
  const [rootHover, setRootHover] = useState(false)
  const [previewVolume, setPreviewVolume] = useState<number>(() => {
    const raw = localStorage.getItem('hardwave.daw.previewVolume')
    const v = raw ? parseFloat(raw) : 0.7
    return Number.isFinite(v) ? Math.max(0, Math.min(1, v)) : 0.7
  })
  const [autoPreview, setAutoPreview] = useState<boolean>(() => {
    return localStorage.getItem('hardwave.daw.autoPreview') === '1'
  })
  const currentAudioRef = useRef<HTMLAudioElement | null>(null)

  useEffect(() => {
    localStorage.setItem('hardwave.daw.previewVolume', String(previewVolume))
    if (currentAudioRef.current) currentAudioRef.current.volume = previewVolume
  }, [previewVolume])

  useEffect(() => {
    localStorage.setItem('hardwave.daw.autoPreview', autoPreview ? '1' : '0')
  }, [autoPreview])

  const q = query.trim().toLowerCase()
  const filter = (p: string) => {
    if (!q) return true
    if (p.toLowerCase().includes(q)) return true
    const tags = fileTags[p]
    if (tags && tags.some(t => t.includes(q))) return true
    return false
  }

  const handleAddTag = (path: string) => {
    const raw = window.prompt('Add tag (comma-separated for multiple)', '')
    if (!raw) return
    raw.split(',').map(t => t.trim()).filter(Boolean).forEach(t => addFileTag(path, t))
  }

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
            autoPreview={autoPreview}
            tags={fileTags[p] ?? []}
            onAddTag={() => handleAddTag(p)}
            onRemoveTag={(t) => removeFileTag(p, t)}
            onClearTags={() => clearFileTags(p)}
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
        <button
          onClick={() => setAutoPreview(v => !v)}
          title={autoPreview
            ? 'Auto-preview ON: click a filename to hear it (double-click to import)'
            : 'Auto-preview OFF: click a filename to import it to the selected track'}
          data-testid="auto-preview-toggle"
          style={{
            padding: '2px 6px', fontSize: 9, fontWeight: 600,
            color: autoPreview ? hw.accent : hw.textFaint,
            background: autoPreview ? 'rgba(124,201,255,0.12)' : 'transparent',
            border: `1px solid ${autoPreview ? hw.accent : hw.border}`,
            borderRadius: hw.radius.sm, cursor: 'pointer',
          }}
        >
          Auto
        </button>
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
              autoPreview={autoPreview}
              tags={fileTags[p] ?? []}
              onAddTag={() => handleAddTag(p)}
              onRemoveTag={(t) => removeFileTag(p, t)}
              onClearTags={() => clearFileTags(p)}
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
            autoPreview={autoPreview}
            tags={fileTags[p] ?? []}
            onAddTag={() => handleAddTag(p)}
            onRemoveTag={(t) => removeFileTag(p, t)}
            onClearTags={() => clearFileTags(p)}
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
        className={`fl-bt-folder${expanded ? ' on' : ''}`}
      >
        <span className="arrow">{expanded ? '▾' : '▸'}</span>
        <span>{label}</span>
        <span className="ct">· {count}</span>
        {actionLabel && onAction && (
          <span
            onClick={(e) => { e.stopPropagation(); onAction() }}
            style={{
              marginLeft: 8,
              fontSize: 9,
              color: hw.textFaint,
              padding: '1px 6px',
              borderRadius: 4,
              background: 'rgba(255,255,255,0.03)',
              fontFamily: hw.font.mono,
              letterSpacing: hw.tracking.eyebrow,
              textTransform: 'uppercase',
              cursor: 'pointer',
            }}
          >
            {actionLabel}
          </span>
        )}
      </div>
      {expanded && <div>{children}</div>}
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
      className="fl-bt-leaf"
      style={{
        cursor: canAdd ? 'pointer' : 'not-allowed',
        opacity: canAdd ? 1 : 0.5,
      }}
      title={`${plugin.name} — ${plugin.vendor} · ${plugin.format}`}
      onClick={async () => {
        if (!canAdd) return
        const { selectedTrackId } = useTrackStore.getState()
        if (!selectedTrackId) return
        await usePluginStore.getState().addToTrack(selectedTrackId, plugin.id)
        useTrackStore.getState().fetchTracks()
      }}
    >
      <span style={{ flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis' }}>
        {plugin.name}
      </span>
      <span className="meta">{plugin.format}</span>
      <button
        type="button"
        onClick={(e) => { e.stopPropagation(); onToggleFavorite() }}
        title={isFavorite ? 'Remove from favorites' : 'Add to favorites'}
        style={{
          width: 14, height: 14, padding: 0,
          background: 'transparent', border: 'none',
          color: isFavorite ? '#fbbf24' : hw.textFaint,
          fontSize: 11, cursor: 'pointer', lineHeight: 1,
          flexShrink: 0,
        }}
      >
        {isFavorite ? '★' : '☆'}
      </button>
    </div>
  )
}

function FileItem({ path, depth = 0, isFavorite, isPreviewing, autoPreview = false, tags = [], onAddTag, onRemoveTag, onClearTags, onToggleFavorite, onPreview, onImport, onRemove }: {
  path: string; depth?: number; isFavorite: boolean; isPreviewing: boolean;
  autoPreview?: boolean;
  tags?: string[];
  onAddTag?: () => void;
  onRemoveTag?: (tag: string) => void;
  onClearTags?: () => void;
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
      draggable
      onDragStart={(e) => {
        e.dataTransfer.setData('application/x-hw-browser', `file:${path}`)
        e.dataTransfer.effectAllowed = isFavorite ? 'copyMove' : 'copy'
      }}
      onContextMenu={(e) => {
        e.preventDefault()
        setCtxMenu({ x: e.clientX, y: e.clientY })
      }}
      style={{
        padding: `3px 8px 3px ${16 + depth * 12}px`,
        display: 'flex', alignItems: 'center', gap: 4,
        transition: 'background 0.15s',
        cursor: 'grab',
        position: 'relative',
      }}
      onMouseEnter={e => { e.currentTarget.style.background = 'rgba(255,255,255,0.06)' }}
      onMouseLeave={e => { e.currentTarget.style.background = 'transparent' }}
    >
      <div
        style={{ flex: 1, minWidth: 0 }}
        onClick={autoPreview ? onPreview : onImport}
        onDoubleClick={autoPreview ? onImport : undefined}
        title={autoPreview
          ? `Preview (double-click to import)\n${path}`
          : `Import to selected track\n${path}`}
      >
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
        {tags.length > 0 && (
          <div style={{
            display: 'flex', flexWrap: 'wrap', gap: 3, marginTop: 2,
          }}>
            {tags.map(t => (
              <span
                key={t}
                onClick={(e) => { e.stopPropagation(); onRemoveTag?.(t) }}
                title={`Remove tag "${t}"`}
                style={{
                  fontSize: 8, padding: '1px 5px',
                  color: hw.accent,
                  background: 'rgba(124,201,255,0.12)',
                  border: `1px solid rgba(124,201,255,0.3)`,
                  borderRadius: 8, cursor: 'pointer',
                  lineHeight: 1.3,
                }}
              >
                #{t}
              </span>
            ))}
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
          <FileMenuItem label="Edit sample…" onClick={() => { setCtxMenu(null); useSampleEditorStore.getState().open(path) }} />
          <FileMenuItem label="Slice sample…" onClick={() => { setCtxMenu(null); useBeatSlicerStore.getState().open(path) }} />
          <div style={{ height: 1, background: hw.border, margin: '3px 0' }} />
          <FileMenuItem label={isFavorite ? 'Remove favorite' : 'Add to favorites'} onClick={() => { setCtxMenu(null); onToggleFavorite() }} />
          <FileMenuItem label="Copy full path" onClick={copyPath} />
          <FileMenuItem label="Copy filename" onClick={copyFilename} />
          {onAddTag && (
            <>
              <div style={{ height: 1, background: hw.border, margin: '3px 0' }} />
              <FileMenuItem label="Add tag…" onClick={() => { setCtxMenu(null); onAddTag() }} />
              {tags.map(t => (
                <FileMenuItem
                  key={`rm-${t}`}
                  label={`Remove tag "${t}"`}
                  onClick={() => { setCtxMenu(null); onRemoveTag?.(t) }}
                />
              ))}
              {tags.length > 1 && onClearTags && (
                <FileMenuItem label="Clear all tags" onClick={() => { setCtxMenu(null); onClearTags() }} />
              )}
            </>
          )}
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
  bpm: number | null
  key: string | null
}

const PITCH_NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'] as const

// Krumhansl-Schmuckler major/minor key profiles.
const MAJOR_PROFILE = [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88]
const MINOR_PROFILE = [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17]

function goertzelMagnitude(samples: Float32Array, start: number, n: number, k: number): number {
  const w = (2 * Math.PI * k) / n
  const coeff = 2 * Math.cos(w)
  let q1 = 0
  let q2 = 0
  for (let i = 0; i < n; i++) {
    const q0 = coeff * q1 - q2 + samples[start + i]
    q2 = q1
    q1 = q0
  }
  const real = q1 - q2 * Math.cos(w)
  const imag = q2 * Math.sin(w)
  return Math.sqrt(real * real + imag * imag)
}

function pearson(a: number[], b: number[]): number {
  const n = a.length
  let sa = 0, sb = 0
  for (let i = 0; i < n; i++) { sa += a[i]; sb += b[i] }
  const ma = sa / n, mb = sb / n
  let num = 0, da = 0, db = 0
  for (let i = 0; i < n; i++) {
    const xa = a[i] - ma
    const xb = b[i] - mb
    num += xa * xb
    da += xa * xa
    db += xb * xb
  }
  const denom = Math.sqrt(da * db)
  return denom === 0 ? 0 : num / denom
}

// Build a 12-bin chromagram via Goertzel at pitch-class frequencies across
// octaves 3-6, then correlate against major/minor key profiles at each of
// the 12 rotations. Returns the best-matching root + mode.
function estimateKey(buffer: AudioBuffer): string | null {
  const sr = buffer.sampleRate
  const ch0 = buffer.getChannelData(0)
  if (ch0.length < sr) return null
  const N = 4096
  const HOP = N
  const maxSamples = Math.min(ch0.length, sr * 8)
  const chroma = new Array(12).fill(0) as number[]
  let frames = 0
  const A4 = 440
  const pitchFreqs: number[] = []
  for (let pc = 0; pc < 12; pc++) {
    for (let oct = 3; oct <= 6; oct++) {
      const midi = pc + 12 * (oct + 1)
      pitchFreqs.push(A4 * Math.pow(2, (midi - 69) / 12))
    }
  }
  const sourceArr = ch0 as Float32Array
  for (let start = 0; start + N <= maxSamples; start += HOP) {
    let idx = 0
    for (let pc = 0; pc < 12; pc++) {
      let mag = 0
      for (let oct = 3; oct <= 6; oct++) {
        const freq = pitchFreqs[idx++]
        const k = (freq * N) / sr
        if (k < 2 || k > N / 2 - 1) continue
        mag += goertzelMagnitude(sourceArr, start, N, k)
      }
      chroma[pc] += mag
    }
    frames++
    if (frames > 64) break
  }
  if (frames === 0) return null
  const total = chroma.reduce((a, b) => a + b, 0)
  if (total === 0) return null
  for (let i = 0; i < 12; i++) chroma[i] /= total
  let bestScore = -Infinity
  let bestLabel: string | null = null
  for (let root = 0; root < 12; root++) {
    const rotated = new Array(12).fill(0) as number[]
    for (let i = 0; i < 12; i++) rotated[i] = chroma[(i + root) % 12]
    const maj = pearson(rotated, MAJOR_PROFILE)
    const min = pearson(rotated, MINOR_PROFILE)
    if (maj > bestScore) { bestScore = maj; bestLabel = `${PITCH_NAMES[root]} maj` }
    if (min > bestScore) { bestScore = min; bestLabel = `${PITCH_NAMES[root]} min` }
  }
  return bestScore > 0.3 ? bestLabel : null
}

// Onset-envelope autocorrelation BPM estimator.
// Downsamples the first channel to a ~100 Hz RMS envelope, takes the
// positive derivative as an onset strength signal, and picks the
// autocorrelation peak between 60 and 200 BPM. Folds stray octaves into
// the 80-160 range so half- or double-time estimates get normalised.
function estimateBpm(buffer: AudioBuffer): number | null {
  const sr = buffer.sampleRate
  const ch = buffer.getChannelData(0)
  if (ch.length < sr) return null
  const ENV_HZ = 100
  const hop = Math.max(1, Math.floor(sr / ENV_HZ))
  const env: number[] = []
  for (let i = 0; i + hop <= ch.length; i += hop) {
    let e = 0
    for (let j = 0; j < hop; j++) {
      const v = ch[i + j]
      e += v * v
    }
    env.push(Math.sqrt(e / hop))
  }
  if (env.length < 100) return null
  const onset: number[] = []
  for (let i = 1; i < env.length; i++) {
    const d = env[i] - env[i - 1]
    onset.push(d > 0 ? d : 0)
  }
  const mean = onset.reduce((a, b) => a + b, 0) / onset.length
  for (let i = 0; i < onset.length; i++) onset[i] -= mean
  const minLag = Math.floor(ENV_HZ * 60 / 200)
  const maxLag = Math.floor(ENV_HZ * 60 / 60)
  let bestLag = -1
  let bestR = -Infinity
  for (let lag = minLag; lag <= maxLag; lag++) {
    let r = 0
    const limit = onset.length - lag
    for (let i = 0; i < limit; i++) r += onset[i] * onset[i + lag]
    if (r > bestR) { bestR = r; bestLag = lag }
  }
  if (bestLag <= 0 || !Number.isFinite(bestR)) return null
  let bpm = (ENV_HZ * 60) / bestLag
  while (bpm < 80) bpm *= 2
  while (bpm > 160) bpm /= 2
  return Math.round(bpm)
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
          bpm: estimateBpm(decoded),
          key: estimateKey(decoded),
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
          {info.bpm !== null && (
            <span title="Estimated BPM from onset autocorrelation" style={{ color: hw.accent }}>
              ~{info.bpm} BPM
            </span>
          )}
          {info.key && (
            <span title="Estimated musical key (chroma vs. Krumhansl-Schmuckler profile)" style={{ color: hw.accent }}>
              {info.key}
            </span>
          )}
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
