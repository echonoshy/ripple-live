import type { ReactNode } from 'react'

export type LibrarySectionProps = {
  label: string
  count: number
  children: ReactNode
}

export function LibrarySection({ label, count, children }: LibrarySectionProps) {
  return (
    <section className="library-section" aria-label={`${label}，${count} 项`}>
      <header>
        <h3>{label}</h3>
        <span>{count}</span>
      </header>
      {children}
    </section>
  )
}
