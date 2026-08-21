import {
  ArrowRight,
  Brain,
  History,
  House,
  ListTodo,
  Mic as Microphone,
  Settings,
  Video as VideoCamera,
} from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import type { ConversationSummary } from '../api'
import { LiveOrb } from './LiveOrb'

const PET_REPLIES = ['我在呀', '要一起做点什么吗？', '抓到我啦', '今天也陪着你'] as const

export function ConversationHome({
  accountLabel,
  recentConversation,
  onStartAudio,
  onStartVideo,
  onOpenMenu,
  onOpenRecent,
  onOpenHistory,
  onOpenMemories,
  onOpenTodos,
  historyError,
}: {
  accountLabel: string
  recentConversation?: ConversationSummary
  onStartAudio(): void
  onStartVideo(): void
  onOpenMenu(): void
  onOpenRecent(): void
  onOpenHistory(): void
  onOpenMemories(): void
  onOpenTodos(): void
  historyError?: string
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
        <button type="button" aria-label="打开设置与更多功能" onClick={onOpenMenu}>
          <Settings aria-hidden="true" />
        </button>
      </header>

      <div className="conversation-home-content">
        <h1>你好，{accountLabel.split('@')[0] || '朋友'}</h1>
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

        <p className="home-companion-prompt">我在这里，今天想一起做什么？</p>

        {historyError && (
          <p className="history-error" role="status">{historyError}</p>
        )}

        {recentConversation && (
          <section className="home-recent" aria-labelledby="home-recent-title">
            <span id="home-recent-title">继续上次对话</span>
            <button type="button" onClick={onOpenRecent}>
              <span className="home-recent-icon" aria-hidden="true"><History /></span>
              <span className="home-recent-copy">
                <strong>{recentConversation.title || '未命名对话'}</strong>
                <small>{recentConversation.preview || '继续刚才的话题'}</small>
              </span>
              <ArrowRight aria-hidden="true" />
            </button>
          </section>
        )}
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

      <nav className="home-navigation" aria-label="主导航">
        <button className="is-active" type="button" aria-current="page">
          <House aria-hidden="true" /><span>陪伴</span>
        </button>
        <button type="button" onClick={onOpenHistory}>
          <History aria-hidden="true" /><span>对话</span>
        </button>
        <button type="button" onClick={onOpenMemories}>
          <Brain aria-hidden="true" /><span>记忆</span>
        </button>
        <button type="button" onClick={onOpenTodos}>
          <ListTodo aria-hidden="true" /><span>待办</span>
        </button>
      </nav>
    </section>
  )
}
