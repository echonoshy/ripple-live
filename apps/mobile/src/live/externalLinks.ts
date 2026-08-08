import { isTauri } from '@tauri-apps/api/core'
import { openUrl } from '@tauri-apps/plugin-opener'

export type ExternalUrlRuntime = {
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
    if (!isExternalHttpUrl(url)) return false

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

export const openExternalUrl = createExternalUrlOpener({
  isNative: isTauri,
  openNative: (url) => openUrl(url),
  openBrowser: (url, target, features) =>
    typeof window === 'undefined' ? null : window.open(url, target, features),
})
