import {
  ClockCounterClockwise,
  Microphone,
  VideoCamera,
} from '@phosphor-icons/react'
import { LiveOrb } from './LiveOrb'

export function ConversationHome({
  onStartAudio,
  onStartVideo,
  onOpenHistory,
  historyError,
}: {
  onStartAudio(): void
  onStartVideo(): void
  onOpenHistory(): void
  historyError?: string
}) {
  return (
    <section className="conversation-home">
      <button
        className="home-history-button"
        type="button"
        aria-label="查看聊天历史"
        onClick={onOpenHistory}
      >
        <ClockCounterClockwise aria-hidden="true" weight="regular" />
      </button>

      <div className="conversation-home-content">
        <div className="home-orb">
          <LiveOrb state="idle" inputLevel={0} outputLevel={0} />
        </div>
        <h1>有什么想聊的？</h1>
        <p className="home-prompt">可以直接说</p>
        <div className="home-actions">
          <button
            className="start-speaking-button"
            type="button"
            onClick={onStartAudio}
          >
            <Microphone aria-hidden="true" weight="regular" />
            开始对话
          </button>
          <button
            className="open-camera-button"
            type="button"
            aria-label="打开镜头"
            onClick={onStartVideo}
          >
            <VideoCamera aria-hidden="true" weight="regular" />
          </button>
        </div>
        {historyError && (
          <p className="history-error" role="status">
            {historyError}
          </p>
        )}
      </div>
    </section>
  )
}
