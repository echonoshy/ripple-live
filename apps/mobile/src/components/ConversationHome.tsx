import {
  ClockCounterClockwise,
  Microphone,
  VideoCamera,
} from '@phosphor-icons/react'

export function ConversationHome({
  onStartAudio,
  onStartVideo,
  onOpenHistory,
}: {
  onStartAudio(): void
  onStartVideo(): void
  onOpenHistory(): void
}) {
  return (
    <section className="conversation-home">
      <button
        className="home-history-button"
        type="button"
        aria-label="查看聊天历史"
        onClick={onOpenHistory}
      >
        <ClockCounterClockwise aria-hidden="true" />
        <span>历史</span>
      </button>

      <div className="conversation-home-content">
        <div className="conversation-core" aria-hidden="true">
          <span />
        </div>
        <h1>想聊点什么？</h1>
        <button
          className="start-speaking-button"
          type="button"
          onClick={onStartAudio}
        >
          <Microphone aria-hidden="true" weight="fill" />
          开始说话
        </button>
        <button
          className="open-camera-button"
          type="button"
          onClick={onStartVideo}
        >
          <VideoCamera aria-hidden="true" />
          打开镜头
        </button>
      </div>
    </section>
  )
}
