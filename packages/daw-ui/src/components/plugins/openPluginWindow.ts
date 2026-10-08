import { invoke } from '@tauri-apps/api/core'
import { useNotificationStore } from '../../stores/notificationStore'
import { layoutFor, windowWidth } from './pluginLayouts'

export interface PluginWindowRequest {
  trackId: string
  slotId: string
  pluginId: string
  pluginName: string
  /**
   * Shown in our own window: a built-in (it has no window of its own) or a
   * plug-in running in its own process (its window cannot show in ours).
   */
  ownWindow: boolean
}

/** The event the main window's PluginPanelHost listens for. */
export const OPEN_PLUGIN_PANEL = 'daw:openPluginPanel'

/** True inside a detached panel window (its label starts "panel-"). */
export function inDetachedWindow(): boolean {
  const meta = (window as unknown as {
    __TAURI_INTERNALS__?: { metadata?: { currentWindow?: { label?: string } } }
  }).__TAURI_INTERNALS__?.metadata
  return (meta?.currentWindow?.label ?? 'main').startsWith('panel-')
}

export function isBuiltIn(pluginId: string): boolean {
  return pluginId.startsWith('hardwave.native.')
}

/**
 * Open a plug-in's window from anywhere (a mixer slot, the channel rack).
 *
 * Built-ins and plug-ins in their own process get our window: floating over
 * the app beside the docked mixer, an OS window of its own beside a detached
 * one. Every other plug-in opens its own editor. The channel rack used to
 * ask for a built-in synth's editor, which built-ins do not have, so
 * clicking one only showed an error.
 */
export function openPluginWindow(req: PluginWindowRequest): void {
  const fail = (e: unknown) => useNotificationStore.getState().push('error', `Could not open ${req.pluginName}: ${String(e)}`)
  if (req.ownWindow) {
    if (inDetachedWindow()) {
      const layout = layoutFor(req.pluginId)
      const q = new URLSearchParams({
        trackId: req.trackId, slotId: req.slotId, pluginId: req.pluginId, name: req.pluginName,
      }).toString()
      invoke('open_panel_window', {
        panel: 'pluginControls', params: q, instance: req.slotId, title: req.pluginName,
        size: layout ? [windowWidth(layout), 640] : null,
      }).catch(fail)
    } else {
      window.dispatchEvent(new CustomEvent<PluginWindowRequest>(OPEN_PLUGIN_PANEL, { detail: req }))
    }
    return
  }
  invoke('open_plugin_editor', {
    pluginId: req.pluginId,
    windowLabel: `plugin-editor:${req.trackId}:${req.slotId}`,
    trackId: req.trackId,
    slotId: req.slotId,
  }).catch((e) => {
    console.error('open_plugin_editor failed', e)
    fail(e)
  })
}
