import { ArrowLeft, type LucideIcon } from 'lucide-react'

export function SecondaryScaffold({
  title,
  icon: Icon,
  onBack,
}: {
  title: string
  icon: LucideIcon
  onBack(): void
}) {
  const description = title === '会议记录'
    ? '之后，音频与视频对话可以在这里沉淀为记录、摘要和行动项。'
    : `${title}能力会在这里逐步完善，并与音频、视频对话自然衔接。`

  return (
    <section className="secondary-scaffold">
      <header className="secondary-scaffold-header">
        <button type="button" aria-label="返回首页" onClick={onBack}>
          <ArrowLeft aria-hidden="true" />
        </button>
        <h1>{title}</h1>
        <span aria-hidden="true" />
      </header>
      <div className="secondary-scaffold-content">
        <span className="secondary-scaffold-icon" aria-hidden="true"><Icon /></span>
        <h2>{title}能力正在搭建中</h2>
        <p>{description}</p>
        <button type="button" onClick={onBack}>返回首页</button>
      </div>
    </section>
  )
}
