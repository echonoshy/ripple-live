import {
  Menu,
  Mic as Microphone,
  Video as VideoCamera,
} from 'lucide-react'
import { LiveOrb } from './LiveOrb'

export function ConversationHome({
  onStartAudio,
  onStartVideo,
  onOpenMenu,
  historyError,
}: {
  onStartAudio(): void
  onStartVideo(): void
  onOpenMenu(): void
  historyError?: string
}) {
  return (
    <section className="conversation-home">
      <header className="conversation-home-header">
        <button type="button" aria-label="打开导航" onClick={onOpenMenu}>
          <Menu aria-hidden="true" />
        </button>
        <strong><span aria-hidden="true" />Ripple Live</strong>
        <span className="conversation-home-header-spacer" aria-hidden="true" />
      </header>

      <div className="conversation-home-content">
        <div className="home-orb">
          <LiveOrb state="idle" inputLevel={0} outputLevel={0} />
        </div>
        <h1>有什么可以帮你？</h1>

        {historyError && (
          <p className="history-error" role="status">
            {historyError}
          </p>
        )}
      </div>

      <footer className="home-composer" aria-label="开始对话">
        <button className="home-video-action" type="button" aria-label="开启视频对话" onClick={onStartVideo}>
          <VideoCamera aria-hidden="true" />
        </button>
        <button className="home-compose-action" type="button" aria-label="开始语音对话" onClick={onStartAudio}>
          <Microphone aria-hidden="true" />
          <span>开始语音对话</span>
        </button>
      </footer>
    </section>
  )
}
