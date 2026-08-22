import {
  Menu,
  Mic as Microphone,
  Video as VideoCamera,
} from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { LiveOrb } from './LiveOrb'

const PET_REPLIES = ['我在呀', '要一起做点什么吗？', '抓到我啦', '今天也陪着你'] as const

export function ConversationHome({
  accountLabel,
  onStartAudio,
  onStartVideo,
  onOpenMenu,
}: {
  accountLabel: string
  onStartAudio(): void
  onStartVideo(): void
  onOpenMenu(): void
}) {
  const [petReply, setPetReply] = useState<string | null>(null)
  const replyTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(() => () => {
    if (replyTimerRef.current) window.clearTimeout(replyTimerRef.current)
  }, [])

  function interactWithPet() {
    const reply = PET_REPLIES[Math.floor(Math.random() * PET_REPLIES.length)]
    setPetReply(reply)
    if (replyTimerRef.current) window.clearTimeout(replyTimerRef.current)
    replyTimerRef.current = window.setTimeout(() => setPetReply(null), 1800)
  }

  return (
    <section className="conversation-home">
      <header className="conversation-home-header">
        <strong className="home-brand"><span aria-hidden="true" />Ripple</strong>
        <button type="button" aria-label="打开菜单" onClick={onOpenMenu}>
          <Menu aria-hidden="true" />
        </button>
      </header>

      <div className="conversation-home-content">
        <h1>你好，{accountLabel.split('@')[0] || '朋友'}</h1>
        <p className="home-companion-prompt">今天想聊点什么？</p>
        <div className="home-orb-stage" aria-label="Ripple 精灵正在等待对话">
          <div className="home-wave-field" aria-hidden="true">
            {Array.from({ length: 13 }, (_, index) => <i key={index} />)}
          </div>
          <div className="home-orb-glow" aria-hidden="true" />
          <button
            className={`home-pet-wander${petReply ? ' is-reacting' : ''}`}
            type="button"
            aria-label="和 Ripple 精灵互动"
            onClick={interactWithPet}
          >
            {petReply ? <span className="home-pet-reply" role="status">{petReply}</span> : null}
            <span className="home-pet-sprite">
              <span className="home-orb">
                <LiveOrb state="idle" inputLevel={0} outputLevel={0} />
              </span>
            </span>
          </button>
          <div className="home-wave-shadow" aria-hidden="true"><i /><i /><i /></div>
        </div>
      </div>

      <div className="home-actions" aria-label="开始对话">
        <button className="home-compose-action" type="button" aria-label="开始语音对话" onClick={onStartAudio}>
          <span className="home-action-rings" aria-hidden="true" />
          <Microphone aria-hidden="true" />
          <span>开始语音</span>
        </button>
        <button className="home-video-action" type="button" aria-label="开启视频对话" onClick={onStartVideo}>
          <VideoCamera aria-hidden="true" />
          <span>视频聊聊</span>
        </button>
      </div>

    </section>
  )
}
