import { isTauri } from '@tauri-apps/api/core'
import {
  isPermissionGranted,
  requestPermission as requestNativeNotificationPermission,
} from '@tauri-apps/plugin-notification'
import {
  ArrowLeft,
  Bell,
  Image as ImageIcon,
  LogOut,
  MessageSquareText,
} from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import type { ChangeEvent } from 'react'
import { UserAvatar } from './UserAvatar'
import './SettingsScreen.css'

type NotificationPermissionState = 'checking' | 'granted' | 'denied' | 'prompt' | 'unavailable'

async function readNotificationPermission(): Promise<NotificationPermissionState> {
  try {
    if (isTauri()) return await isPermissionGranted() ? 'granted' : 'prompt'
    if (typeof Notification === 'undefined') return 'unavailable'
    return Notification.permission === 'default' ? 'prompt' : Notification.permission
  } catch {
    return 'unavailable'
  }
}

async function requestNotificationPermission(): Promise<NotificationPermissionState> {
  try {
    if (isTauri()) {
      return await requestNativeNotificationPermission() === 'granted' ? 'granted' : 'denied'
    }
    if (typeof Notification === 'undefined') return 'unavailable'
    const permission = await Notification.requestPermission()
    return permission === 'default' ? 'prompt' : permission
  } catch {
    return 'unavailable'
  }
}

const notificationCopy: Record<NotificationPermissionState, { status: string; action: string }> = {
  checking: { status: 'CHECKING', action: '检查中' },
  granted: { status: 'ALLOWED', action: '已开启' },
  denied: { status: 'DENIED', action: '重新请求' },
  prompt: { status: 'NOT SET', action: '开启通知' },
  unavailable: { status: 'UNAVAILABLE', action: '当前不可用' },
}

export function SettingsScreen({
  server,
  token,
  email,
  avatarUrl,
  avatarNotice,
  captionsEnabled,
  onBack,
  onAvatarChange,
  onCaptionsChange,
  onSignOut,
}: {
  server: string
  token: string
  email: string
  avatarUrl: string | null
  avatarNotice: string
  captionsEnabled: boolean
  onBack(): void
  onAvatarChange(event: ChangeEvent<HTMLInputElement>): void
  onCaptionsChange(enabled: boolean): void
  onSignOut(): void
}) {
  const avatarInputRef = useRef<HTMLInputElement>(null)
  const [notificationPermission, setNotificationPermission] = useState<NotificationPermissionState>('checking')
  const [notificationBusy, setNotificationBusy] = useState(false)

  useEffect(() => {
    let active = true
    void readNotificationPermission().then((permission) => {
      if (active) setNotificationPermission(permission)
    })
    return () => {
      active = false
    }
  }, [])

  const notification = notificationCopy[notificationPermission]

  const handleNotificationRequest = async () => {
    if (notificationBusy || notificationPermission === 'granted' || notificationPermission === 'unavailable') return
    setNotificationBusy(true)
    setNotificationPermission(await requestNotificationPermission())
    setNotificationBusy(false)
  }

  return (
    <section className="settings-workspace" aria-label="设置">
      <header className="settings-workspace-header">
        <button type="button" aria-label="返回首页" onClick={onBack}>
          <ArrowLeft aria-hidden="true" />
        </button>
        <div aria-label="设置状态">
          <span>SETTINGS</span>
          <i>/</i>
          <strong>READY</strong>
        </div>
      </header>

      <main className="settings-workspace-main">
        <div className="settings-workspace-title">
          <h1>设置</h1>
          <span>SYSTEM / 03</span>
        </div>

        <section className="settings-account" aria-label="账户">
          <button
            className="settings-avatar-button"
            type="button"
            aria-label="选择并更换头像"
            onClick={() => avatarInputRef.current?.click()}
          >
            <UserAvatar server={server} token={token} email={email} avatarUrl={avatarUrl} />
            <span aria-hidden="true"><ImageIcon /></span>
          </button>
          <input
            ref={avatarInputRef}
            className="settings-avatar-input"
            type="file"
            accept="image/jpeg,image/png,image/webp"
            aria-label="选择头像图片"
            onChange={onAvatarChange}
          />
          <div className="settings-account-copy">
            <small>ACCOUNT</small>
            <strong>{email}</strong>
            <button type="button" onClick={() => avatarInputRef.current?.click()}>更换头像</button>
          </div>
        </section>
        {avatarNotice ? <p className="settings-notice" role="status">{avatarNotice}</p> : null}

        <section className="settings-group" aria-labelledby="settings-notification-heading">
          <h2 id="settings-notification-heading">通知与提醒</h2>
          <div className="settings-action-row">
            <span className="settings-row-icon" aria-hidden="true"><Bell /></span>
            <span className="settings-row-copy">
              <strong>待办通知</strong>
              <small>到期时发送系统提醒</small>
            </span>
            <button
              className="settings-permission-button"
              type="button"
              disabled={notificationBusy || notificationPermission === 'granted' || notificationPermission === 'unavailable'}
              onClick={() => void handleNotificationRequest()}
            >
              <span>{notification.status}</span>
              {notificationBusy ? '处理中' : notification.action}
            </button>
          </div>
        </section>

        <section className="settings-group" aria-labelledby="settings-call-heading">
          <h2 id="settings-call-heading">通话显示</h2>
          <div className="settings-action-row">
            <span className="settings-row-icon" aria-hidden="true"><MessageSquareText /></span>
            <span className="settings-row-copy">
              <strong>实时字幕</strong>
              <small>显示你和 Ripple 正在说的内容</small>
            </span>
            <button
              className={`settings-switch ${captionsEnabled ? 'is-active' : ''}`}
              type="button"
              role="switch"
              aria-checked={captionsEnabled}
              aria-label="实时字幕"
              onClick={() => onCaptionsChange(!captionsEnabled)}
            >
              <span />
            </button>
          </div>
        </section>

        <button className="settings-signout" type="button" onClick={onSignOut}>
          <LogOut aria-hidden="true" />
          <span>退出登录</span>
          <small>SIGN OUT</small>
        </button>
      </main>
    </section>
  )
}
