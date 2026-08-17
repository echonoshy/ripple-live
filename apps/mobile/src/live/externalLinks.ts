import { isTauri } from '@tauri-apps/api/core'

export type ExternalUrlRuntime = {
  isIOS(): boolean
  isNative(): boolean
  openNative(url: string): Promise<void>
  openBrowser(
    url: string,
    target: string,
    features: string,
  ): { opener: unknown } | null
}

function isExternalHttpUrl(url: string) {
  try {
    const parsed = new URL(url)
    return (
      (parsed.protocol === 'http:' || parsed.protocol === 'https:') &&
      parsed.hostname.length > 0 &&
      !parsed.username &&
      !parsed.password
    )
  } catch {
    return false
  }
}

export function createExternalUrlOpener(runtime: ExternalUrlRuntime) {
  return async (url: string) => {
    if (runtime.isIOS() || !isExternalHttpUrl(url)) return false

    try {
      if (runtime.isNative()) {
        await runtime.openNative(url)
        return true
      }
      const popup = runtime.openBrowser(
        url,
        '_blank',
        'noopener,noreferrer',
      )
      if (!popup) return false
      popup.opener = null
      return true
    } catch {
      return false
    }
  }
}

function isIOSWebView() {
  if (typeof navigator === 'undefined') return false
  return (
    /iPad|iPhone|iPod/.test(navigator.userAgent) ||
    (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1)
  )
}

export const openExternalUrl = createExternalUrlOpener({
  isIOS: isIOSWebView,
  isNative: isTauri,
  openNative: async (url) => {
    const { openUrl } = await import('@tauri-apps/plugin-opener')
    await openUrl(url)
  },
  openBrowser: (url, target, features) =>
    typeof window === 'undefined' ? null : window.open(url, target, features),
})
