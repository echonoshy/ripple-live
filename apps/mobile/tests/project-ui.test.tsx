import assert from 'node:assert/strict'
import test from 'node:test'
import * as React from 'react'
import { renderToStaticMarkup } from 'react-dom/server'

import type { ConversationSummary, ProjectRecord } from '../src/api.ts'
import { ProjectDetailScreen } from '../src/components/projects/ProjectDetailScreen.tsx'
import { ProjectListScreen } from '../src/components/projects/ProjectListScreen.tsx'
import {
  hasProjectConversationContent,
  projectDraftError,
  projectPayload,
} from '../src/components/projects/projectDraft.ts'

;(globalThis as typeof globalThis & { React: typeof React }).React = React

const project: ProjectRecord = {
  id: 'proj_android',
  name: 'Ripple Android 发布',
  description: '负责 Android APK 的开发与交付。',
  instructions: '优先 Android，不修改 iOS；接口只使用 Responses API。',
  created_at: 1_700_000_000,
  updated_at: 1_700_001_000,
  archived_at: null,
}

const conversation: ConversationSummary = {
  id: 'conv_project',
  title: '发布前检查',
  preview: '继续核对 Android 发布清单。',
  created_at: 1_700_000_100,
  updated_at: 1_700_001_100,
  is_pinned: false,
  archived_at: null,
}

test('normalizes project drafts before API submission', () => {
  const payload = projectPayload({
    name: '  Android 发布  ',
    description: '  负责交付  ',
    instructions: '  只修改 Android  ',
  })

  assert.deepEqual(payload, {
    name: 'Android 发布',
    description: '负责交付',
    instructions: '只修改 Android',
  })
  assert.equal(projectDraftError({ name: '  ', description: '', instructions: '' }), '请输入项目名称')
})

test('hides empty project conversations created by a failed media connection', () => {
  assert.equal(hasProjectConversationContent({
    ...conversation,
    title: '新对话',
    preview: '',
  }), false)
  assert.equal(hasProjectConversationContent(conversation), true)
})

test('renders the project list as a secondary workspace rail', () => {
  const html = renderToStaticMarkup(
    <ProjectListScreen
      items={[project]}
      scope="active"
      busy={false}
      error=""
      onBack={() => {}}
      onScopeChange={() => {}}
      onCreate={() => {}}
      onOpen={() => {}}
      onRetry={() => {}}
    />,
  )

  assert.match(html, /项目/)
  assert.match(html, /进行中/)
  assert.match(html, /Ripple Android 发布/)
  assert.match(html, /负责 Android APK 的开发与交付/)
  assert.doesNotMatch(html, /bottom-navigation/)
})

test('renders project context, call actions, and scoped conversations', () => {
  const html = renderToStaticMarkup(
    <ProjectDetailScreen
      project={project}
      conversations={[conversation]}
      busy={false}
      callBusy={false}
      error=""
      onBack={() => {}}
      onEdit={() => {}}
      onArchive={() => {}}
      onRestore={() => {}}
      onStartAudio={() => {}}
      onStartVideo={() => {}}
      onOpenConversation={() => {}}
      onRetry={() => {}}
    />,
  )

  assert.match(html, /PROJECT\.KIRO/)
  assert.match(html, /只使用 Responses API/)
  assert.match(html, /开始语音/)
  assert.match(html, /视频聊聊/)
  assert.match(html, /发布前检查/)
})

test('archived projects replace call controls with a restore action', () => {
  const html = renderToStaticMarkup(
    <ProjectDetailScreen
      project={{ ...project, archived_at: 1_700_002_000 }}
      conversations={[]}
      busy={false}
      callBusy={false}
      error=""
      onBack={() => {}}
      onEdit={() => {}}
      onArchive={() => {}}
      onRestore={() => {}}
      onStartAudio={() => {}}
      onStartVideo={() => {}}
      onOpenConversation={() => {}}
      onRetry={() => {}}
    />,
  )

  assert.match(html, /项目已归档/)
  assert.match(html, /恢复/)
  assert.doesNotMatch(html, /project-call-actions/)
})
