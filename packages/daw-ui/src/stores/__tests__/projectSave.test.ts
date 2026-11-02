import { describe, it, expect, vi, beforeEach } from 'vitest'

// Stands in for the backend: every command succeeds and reads come back empty.
const invokeMock = vi.fn(async (_cmd: string, _args?: Record<string, unknown>) => null as unknown)
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args?: Record<string, unknown>) => invokeMock(cmd, args),
}))
// The save dialog: what the producer picks, or null for Cancel.
const saveDialog = vi.fn(async (_opts: { defaultPath?: string }) => null as string | null)
vi.mock('@tauri-apps/plugin-dialog', () => ({
  save: (opts: { defaultPath?: string }) => saveDialog(opts),
}))

const store = new Map<string, string>()
vi.stubGlobal('localStorage', {
  getItem: (k: string) => store.get(k) ?? null,
  setItem: (k: string, v: string) => void store.set(k, v),
  removeItem: (k: string) => void store.delete(k),
  clear: () => store.clear(),
})

const { useProjectStore } = await import('../projectStore')

const savedTo = () =>
  invokeMock.mock.calls.filter(([cmd]) => cmd === 'save_project').map(([, args]) => args?.path)

beforeEach(() => {
  invokeMock.mockClear()
  saveDialog.mockReset()
  saveDialog.mockResolvedValue(null)
  store.clear()
  useProjectStore.setState({ filePath: null, projectName: 'Untitled', dirty: false, recentProjects: [] })
})

describe('Save As', () => {
  it('asks where even when the song already has a file, and moves the song there', async () => {
    useProjectStore.setState({ filePath: '/songs/Anthem.hwp', projectName: 'Anthem', dirty: true })
    saveDialog.mockResolvedValue('/songs/Anthem remix.hwp')

    await useProjectStore.getState().saveProjectAs()

    expect(saveDialog).toHaveBeenCalledTimes(1)
    expect(saveDialog.mock.calls[0][0].defaultPath).toBe('/songs/Anthem.hwp')
    expect(savedTo()).toEqual(['/songs/Anthem remix.hwp'])
    const s = useProjectStore.getState()
    expect(s.filePath).toBe('/songs/Anthem remix.hwp')
    expect(s.projectName).toBe('Anthem remix')
    expect(s.dirty).toBe(false)
  })

  it('writes nothing when the dialog is cancelled', async () => {
    useProjectStore.setState({ filePath: '/songs/Anthem.hwp', projectName: 'Anthem', dirty: true })

    await useProjectStore.getState().saveProjectAs()

    expect(savedTo()).toEqual([])
    expect(useProjectStore.getState().filePath).toBe('/songs/Anthem.hwp')
    expect(useProjectStore.getState().dirty).toBe(true)
  })

  it('a plain save of a saved song still goes to its own file without asking', async () => {
    useProjectStore.setState({ filePath: '/songs/Anthem.hwp', projectName: 'Anthem', dirty: true })

    await useProjectStore.getState().saveProject()

    expect(saveDialog).not.toHaveBeenCalled()
    expect(savedTo()).toEqual(['/songs/Anthem.hwp'])
  })
})
