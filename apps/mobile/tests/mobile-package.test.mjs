import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { existsSync, readFileSync } from 'node:fs'
import path from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'

const appRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const sourceHash = '78dccdf08b07a642c2fbbdce7a1d965b8935512d7465b34a7790b4d09b6aad4f'

function readPngInfo(file) {
  const bytes = readFileSync(file)
  assert.equal(bytes.toString('ascii', 1, 4), 'PNG', `${file} must be a PNG`)
  return {
    width: bytes.readUInt32BE(16),
    height: bytes.readUInt32BE(20),
    colorType: bytes[25],
  }
}

function expectSquare(file, size) {
  const { width, height } = readPngInfo(file)
  assert.deepEqual({ width, height }, { width: size, height: size }, file)
}

function expectRgba(file) {
  assert.equal(readPngInfo(file).colorType, 6, `${file} must be RGBA`)
}

test('mobile package has the supplied icon and iOS media permissions', () => {
  const source = path.join(appRoot, 'src-tauri/icons/ripple-live-source.png')
  const sourceBytes = readFileSync(source)
  assert.equal(createHash('sha256').update(sourceBytes).digest('hex'), sourceHash)
  const { width, height } = readPngInfo(source)
  assert.deepEqual({ width, height }, { width: 1024, height: 1024 })
  expectRgba(source)

  expectSquare(path.join(appRoot, 'src-tauri/icons/icon.png'), 512)
  expectRgba(path.join(appRoot, 'src-tauri/icons/32x32.png'))
  const androidRoot = path.join(
    appRoot,
    'src-tauri/gen/android/app/src/main/res/mipmap-xxxhdpi',
  )
  expectSquare(path.join(androidRoot, 'ic_launcher.png'), 192)
  expectSquare(path.join(androidRoot, 'ic_launcher_round.png'), 192)

  const appleRoot = path.join(
    appRoot,
    'src-tauri/gen/apple/Assets.xcassets/AppIcon.appiconset',
  )
  const manifest = JSON.parse(
    readFileSync(path.join(appleRoot, 'Contents.json'), 'utf8'),
  )
  for (const image of manifest.images) {
    const [points] = image.size.split('x').map(Number)
    const scale = Number.parseInt(image.scale, 10)
    expectSquare(path.join(appleRoot, image.filename), Math.round(points * scale))
  }

  const tauriConfig = JSON.parse(
    readFileSync(path.join(appRoot, 'src-tauri/tauri.conf.json'), 'utf8'),
  )
  const iosConfig = JSON.parse(
    readFileSync(path.join(appRoot, 'src-tauri/tauri.ios.conf.json'), 'utf8'),
  )
  assert.equal(tauriConfig.identifier, 'cn.minicpm.live')
  assert.equal(iosConfig.identifier, 'cn.minicpm.ripplelive.debug')
  assert.equal(iosConfig.bundle.iOS.developmentTeam, '932QX878KY')

  for (const plistPath of [
    path.join(appRoot, 'src-tauri/Info.ios.plist'),
    path.join(appRoot, 'src-tauri/gen/apple/ripple-live_iOS/Info.plist'),
  ]) {
    const plist = readFileSync(plistPath, 'utf8')
    assert.match(plist, /<key>NSCameraUsageDescription<\/key>/)
    assert.match(plist, /<key>NSMicrophoneUsageDescription<\/key>/)
    assert.match(plist, /Ripple Live 使用相机进行实时视频通话。/)
    assert.match(plist, /Ripple Live 使用麦克风进行实时语音通话。/)
    assert.match(plist, /<key>NSAppTransportSecurity<\/key>/)
    assert.match(plist, /<key>NSExceptionDomains<\/key>/)
    assert.match(plist, /<key>140\.143\.229\.103<\/key>/)
    assert.match(plist, /<key>NSExceptionAllowsInsecureHTTPLoads<\/key>/)
  }
})

test('mobile app keeps the service address out of visible forms', () => {
  const appSource = readFileSync(path.join(appRoot, 'src/App.tsx'), 'utf8')
  assert.doesNotMatch(appSource, /htmlFor="auth-server"/)
  assert.doesNotMatch(appSource, /htmlFor="server">服务地址/)
})

test('settings exposes persistent personalization separately from visual memory', () => {
  const appSource = readFileSync(path.join(appRoot, 'src/App.tsx'), 'utf8')
  const apiSource = readFileSync(path.join(appRoot, 'src/api.ts'), 'utf8')
  const profileSource = readFileSync(
    path.join(appRoot, 'src/components/PersonalizationSection.tsx'),
    'utf8',
  )
  const cssSource = readFileSync(path.join(appRoot, 'src/App.css'), 'utf8')

  assert.match(appSource, /<PersonalizationSection server=\{server\} token=\{accessToken\} \/>/)
  assert.match(apiSource, /'\/v1\/profile'/)
  for (const label of ['Ripple 的身份', '你的身份', '希望怎么称呼你', '基础资料']) {
    assert.match(profileSource, new RegExp(label))
  }
  assert.match(profileSource, /不会混入通话里保存的视觉记忆/)
  assert.match(profileSource, /已保存，将从下一轮对话开始生效/)
  assert.match(
    cssSource,
    /\.personalization-form input,[\s\S]*?min-height:\s*48px;/,
  )
  assert.match(
    cssSource,
    /\.personalization-form button\s*{[^}]*min-height:\s*48px;/s,
  )
})

test('mobile uses protocol v5 semantic endpointing and mode changes', () => {
  const appSource = readFileSync(path.join(appRoot, 'src/App.tsx'), 'utf8')
  const mediaSource = readFileSync(
    path.join(appRoot, 'src/media/LiveMedia.ts'),
    'utf8',
  )
  const playbackSource = readFileSync(
    path.join(appRoot, 'public/playback-processor.js'),
    'utf8',
  )
  const realtimeSource = readFileSync(
    path.join(appRoot, 'src/realtime/RealtimeSession.ts'),
    'utf8',
  )
  const protocolSource = readFileSync(
    path.join(appRoot, 'src/realtime/protocol.ts'),
    'utf8',
  )

  assert.doesNotMatch(appSource, /ripple-activation-mode/)
  assert.doesNotMatch(appSource, /静默唤醒/)
  assert.doesNotMatch(appSource, /ActivationMode/)
  assert.match(mediaSource, /private preRoll: Float32Array\[\]/)
  assert.match(mediaSource, /if \(this\.speechActive\) \{\s*onChunk\(audio, null\)/s)
  assert.doesNotMatch(mediaSource, /onChunk\(audio, this\.captureFrame\(\)\)/)
  assert.doesNotMatch(realtimeSource, /input\.activation/)
  assert.doesNotMatch(realtimeSource, /session\.wake/)
  assert.doesNotMatch(realtimeSource, /forceWake/)
  assert.match(realtimeSource, /case 'input\.frame\.requested'/)
  assert.match(realtimeSource, /'high'/)
  assert.match(realtimeSource, /onInterrupted: \(\) => void/)
  assert.match(
    appSource,
    /onInterrupted: \(\) => \{\s*if \(!ownsSession\(\)\) return\s*media\.clearOutput\(\)\s*const signal = createRippleSignal\('interrupt'\)\s*setRippleSignals\(\(current\) => enqueueRippleSignal\(current, signal\)\)/,
  )
  assert.match(protocolSource, /REALTIME_PROTOCOL_VERSION = 5/)
  assert.match(protocolSource, /type: 'session\.mode\.set'/)
  assert.match(protocolSource, /mode !== 'audio' && mode !== 'video'/)
  assert.match(appSource, /void session\.speechPaused\(\)/)
  assert.doesNotMatch(appSource, /void session\.commitInput\(\)/)
  assert.match(realtimeSource, /setTimeout\(\(\) => \{[\s\S]*?\}, 1_500\)/)
  assert.match(mediaSource, /width: \{ ideal: 1280 \}/)
  assert.match(mediaSource, /height: \{ ideal: 720 \}/)
  assert.doesNotMatch(mediaSource, /lowPower/)
  assert.match(playbackSource, /type: 'audio-level'/)
  assert.match(mediaSource, /onOutputLevel: \(level: number\) => void/)
  assert.match(mediaSource, /this\.options\.onOutputLevel\(event\.data\.level\)/)
})

test('every live session UI and media callback checks active ownership', () => {
  const appSource = readFileSync(path.join(appRoot, 'src/App.tsx'), 'utf8')
  const callbackSource = appSource.slice(
    appSource.indexOf('const ownsSession ='),
    appSource.indexOf('mediaRef.current = media'),
  )

  for (const callback of [
    'onPlaybackStarted',
    'onPlaybackEnded',
    'onOutputLevel',
    'onState',
    'onError',
    'onResponseFailed',
    'onAssistantText',
    'onUserText',
    'onTool',
    'onToolResult',
    'onAudio',
    'onAudioDone',
    'onInterrupted',
    'onFrameRequested',
    'onArtifact',
    'onReady',
  ]) {
    const start = callbackSource.indexOf(`${callback}:`)
    assert.notEqual(start, -1, `${callback} should be configured`)
    const next = callbackSource.indexOf('\n        on', start + callback.length + 1)
    const body = callbackSource.slice(start, next === -1 ? undefined : next)
    assert.match(body, /ownsSession\(\)/, `${callback} should guard session ownership`)
  }
})

test('mobile todo reminders are enabled for Android and iOS', () => {
  const appSource = readFileSync(path.join(appRoot, 'src/App.tsx'), 'utf8')
  const reminderSource = readFileSync(path.join(appRoot, 'src/reminders.ts'), 'utf8')
  const rustSource = readFileSync(path.join(appRoot, 'src-tauri/src/lib.rs'), 'utf8')
  const capability = JSON.parse(
    readFileSync(path.join(appRoot, 'src-tauri/capabilities/default.json'), 'utf8'),
  )

  assert.match(appSource, /notifyDueTodos/)
  assert.match(appSource, /screen === 'todos'/)
  assert.match(reminderSource, /requestPermission/)
  assert.match(reminderSource, /sendNotification/)
  assert.match(rustSource, /tauri_plugin_notification::init\(\)/)
  assert.ok(capability.permissions.includes('notification:default'))
})

test('mobile todos retain completed items in a dedicated view', () => {
  const appSource = readFileSync(path.join(appRoot, 'src/App.tsx'), 'utf8')
  const cssSource = readFileSync(path.join(appRoot, 'src/App.css'), 'utf8')

  assert.match(appSource, /const \[todoView, setTodoView\] = useState<'active' \| 'completed'>\('active'\)/)
  assert.match(appSource, /todos\(server, accessToken, todoView === 'completed'\)/)
  assert.match(appSource, /\n\s*进行中\n\s*<\/button>/)
  assert.match(appSource, /\n\s*已完成\n\s*<\/button>/)
  assert.doesNotMatch(appSource, /className="todo-intro"/)
  assert.match(appSource, /完成：\$\{formatHistoryTime\(todo\.completed_at\)\}/)
  assert.match(appSource, /onPointerMove=\{moveTodoGesture\}/)
  assert.match(appSource, /baseOffset: revealedTodo === id \? 74 : 0/)
  assert.match(appSource, /setRevealedTodo\(offset >= 37 \? start\.id : null\)/)
  assert.match(appSource, /aria-label=\{`删除：\$\{todo\.title\}`\}/)
  assert.match(cssSource, /\.todo-swipe-shell\.is-dragging \.todo-card-surface/)
  assert.match(cssSource, /\.todo-view-switch/)
})

test('conversation history exposes persisted memory and todo actions safely', () => {
  const appSource = readFileSync(path.join(appRoot, 'src/App.tsx'), 'utf8')
  const apiSource = readFileSync(path.join(appRoot, 'src/api.ts'), 'utf8')
  const actionPath = path.join(appRoot, 'src/components/ConversationActions.tsx')
  const actionBehaviorPath = path.join(appRoot, 'src/conversationActions.ts')

  assert.equal(existsSync(actionPath), true)
  assert.equal(existsSync(actionBehaviorPath), true)
  const actionSource = readFileSync(actionPath, 'utf8')
  const actionBehaviorSource = readFileSync(actionBehaviorPath, 'utf8')
  assert.match(apiSource, /actions: ConversationAction\[\]/)
  assert.match(appSource, /<ConversationActions/)
  assert.match(actionBehaviorSource, /kind === 'memory'/)
  assert.match(actionBehaviorSource, /kind !== 'todo'/)
  assert.doesNotMatch(actionSource, /dangerouslySetInnerHTML/)
})

test('mobile home presents voice as the primary call entry with explicit camera access', () => {
  const appSource = readFileSync(path.join(appRoot, 'src/App.tsx'), 'utf8')
  const apiSource = readFileSync(path.join(appRoot, 'src/api.ts'), 'utf8')
  const homePath = path.join(appRoot, 'src/components/ConversationHome.tsx')
  const cssSource = readFileSync(path.join(appRoot, 'src/App.css'), 'utf8')
  const navigationCssSource = readFileSync(path.join(appRoot, 'src/components/AppNavigation.css'), 'utf8')

  assert.equal(existsSync(homePath), true, 'ConversationHome.tsx should exist')
  const homeSource = readFileSync(homePath, 'utf8')
  assert.match(appSource, /<ConversationHome/)
  assert.match(homeSource, /我在这里，今天想一起做什么？/)
  assert.match(homeSource, /accountLabel/)
  assert.match(homeSource, /开始语音/)
  assert.match(homeSource, /aria-label="开始语音对话"/)
  assert.equal((homeSource.match(/<LiveOrb\b/g) ?? []).length, 1)
  assert.match(homeSource, /<LiveOrb\s+state="idle"/)
  assert.match(homeSource, /继续上次对话/)
  assert.match(homeSource, /aria-label="开启视频对话"/)
  assert.equal((homeSource.match(/onClick=\{onStartAudio\}/g) ?? []).length, 1)
  assert.match(homeSource, /onClick=\{onStartVideo\}/)
  assert.match(homeSource, /onOpenHistory|onOpenMemories|onOpenTodos/)
  assert.match(appSource, /onStartAudio=\{\(\) => openCall\('audio'\)\}/)
  assert.match(appSource, /onStartVideo=\{\(\) => openCall\('video'\)\}/)
  assert.match(apiSource, /export async function conversation\(/)
  assert.match(appSource, /conversationId:\s*activeConversationId/)
  assert.match(appSource, /onConversation:\s*setActiveConversationId/)
  assert.match(appSource, /openCall\('audio', selectedConversation\.id\)/)
  assert.doesNotMatch(appSource, /打开镜头，开始聊聊/)
  assert.doesNotMatch(homeSource, /统计|最近对话|自动保存/)
  assert.doesNotMatch(homeSource, /conversation-core/)
  assert.doesNotMatch(cssSource, /\.conversation-core/)
  assert.match(navigationCssSource, /\.home-orb-stage\s*\{[^}]*height:\s*clamp\(230px, 34dvh, 300px\)/s)
  assert.doesNotMatch(homeSource, /home-orbit|home-stage-floor/)
  assert.match(homeSource, /home-wave-field/)
  assert.match(homeSource, /aria-label="和 Ripple 精灵互动"/)
  assert.doesNotMatch(homeSource, /setInterval\(\(\) =>/)
  assert.match(navigationCssSource, /\.home-navigation\s*\{/)
  assert.doesNotMatch(cssSource, /#9046ff|--ripple-violet|--voice-accent:\s*#b98aff/)
})

test('mobile uses the approved nocturne commercial tokens and typography', () => {
  const cssSource = readFileSync(path.join(appRoot, 'src/App.css'), 'utf8')
  const indexSource = readFileSync(path.join(appRoot, 'src/index.css'), 'utf8')
  const navigationCssSource = readFileSync(
    path.join(appRoot, 'src/components/AppNavigation.css'),
    'utf8',
  )

  for (const token of [
    '--font-ui: "SF Pro Text"',
    '--font-display: "SF Pro Display"',
    '--font-brand: ui-serif',
    '--weight-regular: 400',
    '--weight-medium: 500',
    '--weight-semibold: 600',
    '--weight-bold: 700',
    '--live-bg: #080a14',
    '--app-bg: #070910',
    '--surface: #12141f',
    '--surface-raised: #191b29',
    '--text-primary: #fff2da',
    '--danger: #f0797d',
    '--success: #76b795',
    '--line: rgb(255 222 169 / 13%)',
    '--orb-cobalt: #8981db',
    '--orb-soft-blue: #efad5e',
    '--orb-cream: #fff2d8',
    '--focus-ring: rgb(255 207 121 / 48%)',
  ]) {
    assert.match(cssSource, new RegExp(token.replace(/[()]/g, '\\$&')))
  }
  assert.match(
    cssSource,
    /font-family:\s*var\(--font-ui\);/,
  )
  assert.doesNotMatch(cssSource, /body\s*\{[^}]*letter-spacing:\s*-/s)
  assert.match(indexSource, /background:\s*#070910/)
  assert.match(navigationCssSource, /\.home-wave-field\s*\{/)
  assert.match(navigationCssSource, /Nocturne interaction layer/)
  assert.match(navigationCssSource, /Warm commercial home and navigation chrome/)
  assert.match(navigationCssSource, /@keyframes drawer-enter/)
})

test('live call owns camera transitions explicitly and renders truthful camera states', () => {
  const appSource = readFileSync(path.join(appRoot, 'src/App.tsx'), 'utf8')
  const mediaSource = readFileSync(path.join(appRoot, 'src/media/LiveMedia.ts'), 'utf8')
  const callSource = readFileSync(
    path.join(appRoot, 'src/components/LiveCallScreen.tsx'),
    'utf8',
  )
  const callCssSource = readFileSync(path.join(appRoot, 'src/live/LiveCall.css'), 'utf8')
  const presentationSource = readFileSync(
    path.join(appRoot, 'src/live/callPresentation.ts'),
    'utf8',
  )

  assert.match(appSource, /createCameraOrchestrator/)
  assert.match(appSource, /initialCameraRequestRef\.current = nextMode === 'video'/)
  assert.match(appSource, /onFrameRequestState: \(active\)/)
  assert.match(appSource, /cameraControlReady/)
  assert.match(appSource, /if \(!cameraControlReadyRef\.current\) return/)
  assert.match(
    appSource,
    /const activationToken = cameraActivation\.begin\(\)[\s\S]*await media\.start[\s\S]*cameraActivation\.commit\(activationToken\)/,
  )
  assert.match(appSource, /mediaRef\.current === media/)
  assert.match(appSource, /cameraOrchestratorRef\.current === cameraOrchestrator/)
  for (const blockedState of ['idle', 'connecting', 'preparing', 'ended', 'error']) {
    assert.match(appSource, new RegExp(`state === '${blockedState}'`))
  }
  assert.doesNotMatch(appSource, /withVideo:/)
  assert.doesNotMatch(mediaSource, /withVideo/)
  assert.match(callSource, /cameraPhase === 'opening'/)
  assert.match(callSource, /frameRequestActive && cameraPhase === 'on'/)
  assert.match(callSource, /liveCallLabels\(state, cameraPhase, toolStatus\)/)
  assert.match(callSource, /className="live-state-label"[\s\S]*\{labels\.primary\}/)
  assert.match(callSource, /className="live-camera-label"[\s\S]*\{labels\.camera\}/)
  assert.match(presentationSource, /cameraPhase === 'opening'[\s\S]*'正在开启镜头'/)
  assert.match(presentationSource, /cameraPhase === 'on'[\s\S]*'镜头已开启'/)
  assert.doesNotMatch(presentationSource, /frameRequestActive|正在识别/)
  assert.match(callSource, /disabled=\{!cameraControlReady \|\| cameraBusy\}/)
  assert.match(presentationSource, /case 'closing':[\s\S]*'正在关闭镜头'/)
  assert.match(presentationSource, /case 'error':[\s\S]*'重试镜头'/)
  assert.doesNotMatch(callSource, /cameraHeaderAction\(|headerAction/)
  assert.match(callSource, /videoMode \? \([\s\S]*aria-label="切换前后摄像头"/)
  assert.match(callSource, /onFlipCamera\(\)\.catch/)
  assert.match(callSource, /<span className="call-header-spacer"/)
  assert.match(callCssSource, /opacity 420ms/)
  assert.match(callCssSource, /prefers-reduced-motion: reduce/)
})

test('mobile navigation uses a screen-derived side drawer without a bottom bar', () => {
  const appSource = readFileSync(path.join(appRoot, 'src/App.tsx'), 'utf8')
  const navPath = path.join(appRoot, 'src/components/AppDrawer.tsx')
  const navCssPath = path.join(appRoot, 'src/components/AppNavigation.css')

  assert.equal(existsSync(navPath), true, 'AppDrawer.tsx should exist')
  assert.equal(existsSync(navCssPath), true, 'AppNavigation.css should exist')
  const navSource = readFileSync(navPath, 'utf8')
  const navCssSource = readFileSync(navCssPath, 'utf8')
  for (const label of ['开始', '对话历史', '记忆', '待办', '设置']) {
    assert.match(navSource, new RegExp(label))
  }
  assert.match(navSource, /export type AppDestination/)
  assert.match(navSource, /aria-label="Ripple 功能"/)
  assert.match(navSource, /aria-current=\{active === destination \? 'page' : undefined\}/)
  assert.match(appSource, /case 'conversation':\s*return 'home'/)
  assert.match(appSource, /case 'history':\s*return 'history'/)
  assert.match(appSource, /case 'memories':\s*return 'memories'/)
  assert.match(appSource, /case 'todos':\s*return 'todos'/)
  assert.match(appSource, /case 'settings':\s*return 'settings'/)
  assert.equal((appSource.match(/library-sticky-header/g) ?? []).length, 3)
  assert.match(appSource, /<AppDrawer/)
  assert.doesNotMatch(appSource, /<BottomNav/)
  assert.doesNotMatch(appSource, /with-bottom-nav/)
  assert.match(appSource, /window\.requestAnimationFrame\(\(\) => window\.scrollTo\(0, 0\)\)/)
  assert.equal((appSource.match(/<AppDrawer/g) ?? []).length, 1)
  assert.match(navCssSource, /\.app-drawer\s*\{/)
  assert.match(navCssSource, /height:\s*100dvh/)
  assert.match(navCssSource, /env\(safe-area-inset-bottom\)/)
  assert.match(navSource, /type LucideIcon/)
  assert.match(navSource, /<Icon aria-hidden="true" \/>/)
  assert.doesNotMatch(navSource, /weight=/)
  assert.match(navCssSource, /\.drawer-navigation > button\.is-active/)
  assert.match(navSource, /event\.key === 'Escape'/)
  assert.doesNotMatch(navSource, /drawer-new-conversation|onNewConversation/)
})

test('mobile live pet uses a single canvas renderer with a static fallback', () => {
  for (const file of [
    'live/petRenderer.ts',
    'components/LiveOrb.tsx',
    'live/LiveCall.css',
    'assets/starry-avatar.webp',
    'assets/starry-avatar-states@2x.png',
    'assets/pet-gifs/starry-avatar-idle.gif',
    'assets/pet-gifs/starry-avatar-waving.gif',
    'assets/pet-gifs/starry-avatar-failed.gif',
    'assets/pet-gifs/starry-avatar-waiting.gif',
    'assets/pet-gifs/starry-avatar-running.gif',
    'assets/pet-gifs/starry-avatar-review.gif',
  ]) {
    assert.equal(
      existsSync(path.join(appRoot, 'src', file)),
      true,
      `${file} should exist`,
    )
  }

  const orbSource = readFileSync(
    path.join(appRoot, 'src/components/LiveOrb.tsx'),
    'utf8',
  )
  assert.equal((orbSource.match(/<canvas/g) ?? []).length, 1)
  assert.match(
    orbSource,
    /createPetRenderer\(\s*canvas,\s*starryAvatarUrl,\s*starryAvatarHdUrl,\s*starryAvatarGifUrls,\s*\)/,
  )
  const homeSource = readFileSync(
    path.join(appRoot, 'src/components/ConversationHome.tsx'),
    'utf8',
  )
  const navigationCssSource = readFileSync(
    path.join(appRoot, 'src/components/AppNavigation.css'),
    'utf8',
  )
  assert.doesNotMatch(homeSource, /running-right|running-left|petPosition|wanderTimer/)
  assert.doesNotMatch(navigationCssSource, /--pet-facing|scaleX\(var\(--pet-facing/)
  assert.doesNotMatch(navigationCssSource, /live-orb-directional-gif|--pet-x|--pet-y/)
  assert.doesNotMatch(orbSource, /lottie|video/i)
})

test('mobile live pet fallback uses the packaged atlas without a circular crop', () => {
  const cssSource = readFileSync(
    path.join(appRoot, 'src/live/LiveCall.css'),
    'utf8',
  )
  const fallbackRule = [...cssSource.matchAll(/^\.live-orb-fallback\s*\{([^}]*)\}/gm)]
    .map((match) => match[1])
    .find((body) => body.includes('starry-avatar.webp'))

  assert.ok(fallbackRule, 'fallback pet rule should exist')
  assert.match(fallbackRule, /starry-avatar\.webp/)
  assert.match(fallbackRule, /background-size:\s*800% 1100%/)
  assert.match(cssSource, /aspect-ratio:\s*192 \/ 208/)
  assert.match(cssSource, /border-radius:\s*0/)
})

test('mobile live pet fallback has no legacy orb material pseudo-elements', () => {
  const cssSource = readFileSync(
    path.join(appRoot, 'src/live/LiveCall.css'),
    'utf8',
  )
  assert.doesNotMatch(cssSource, /\.live-orb-fallback::before/)
  assert.doesNotMatch(cssSource, /\.live-orb-fallback::after/)
})

test('mobile live call uses the immersive presentation contract', () => {
  const appSource = readFileSync(path.join(appRoot, 'src/App.tsx'), 'utf8')
  const callSource = readFileSync(
    path.join(appRoot, 'src/components/LiveCallScreen.tsx'),
    'utf8',
  )
  const callCssSource = readFileSync(
    path.join(appRoot, 'src/live/LiveCall.css'),
    'utf8',
  )
  const presentationSource = readFileSync(
    path.join(appRoot, 'src/live/callPresentation.ts'),
    'utf8',
  )

  assert.match(callSource, /<LiveOrb/)
  assert.match(callSource, /<LiveCaption/)
  assert.doesNotMatch(callSource, /HandPalm|打断回答/)
  assert.doesNotMatch(callSource, /PhoneDisconnect/)
  assert.doesNotMatch(callSource, /className="call-status"|className="call-mode"/)
  assert.match(presentationSource, /listening: ''/)
  assert.match(presentationSource, /thinking: '想一想'/)
  assert.match(presentationSource, /speaking: ''/)
  assert.match(presentationSource, /error: '连接断开'/)
  assert.doesNotMatch(callSource, /aria-label="收起通话"|<CaretDown/)
  assert.match(callSource, /className="icon-button call-icon call-back"/)
  assert.match(callSource, /正在陪伴/)
  assert.match(callSource, /\{formatDuration\(elapsed\)\}/)
  assert.match(callSource, /aria-label="切换前后摄像头"/)
  assert.match(callSource, /aria-label=\{muted \? '取消静音' : '静音'\}/)
  assert.match(callSource, /aria-label="结束通话"/)
  const controlsStart = callSource.indexOf('<footer className="call-controls">')
  const controlsEnd = callSource.indexOf('</footer>', controlsStart)
  const controlsSource = callSource.slice(controlsStart, controlsEnd)
  assert.ok(controlsStart >= 0 && controlsEnd > controlsStart)
  assert.match(
    controlsSource,
    /aria-label=\{muted \? '取消静音' : '静音'\}[\s\S]*aria-label="结束通话"[\s\S]*开启镜头/,
  )
  assert.match(controlsSource, /className="end-button"[\s\S]*<Phone aria-hidden="true"/)
  assert.match(callSource, /className="call-control-item"/)
  assert.match(callSource, /<span>\{muted \? '取消静音' : '静音'\}<\/span>/)
  const controlsRule = [...callCssSource.matchAll(
    /\.live-call-screen \.call-controls\s*\{([^}]*)\}/g,
  )].at(-1)?.[1] ?? ''
  assert.doesNotMatch(controlsRule, /background:|border:|backdrop-filter:/)
  assert.match(callCssSource, /\.live-call-screen \.control-button,\s*\.live-call-screen \.end-button\s*\{[^}]*width:\s*58px;[^}]*height:\s*58px;/s)
  assert.match(callCssSource, /\.call-control-item\s*\{/)
  assert.match(callCssSource, /\.live-call-screen \.end-button > svg\s*\{[^}]*rotate\(135deg\)/s)
  assert.match(
    appSource,
    /setUserText\(''\)\s*setAssistantText\(''\)\s*const signal = createRippleSignal\('speech'\)\s*setRippleSignals\(\(current\) => enqueueRippleSignal\(current, signal\)\)\s*void session\.speechStarted\(\)/,
  )
})

test('mobile emits Ripple signals only from confirmed live events', () => {
  const appSource = readFileSync(path.join(appRoot, 'src/App.tsx'), 'utf8')
  const callSource = readFileSync(
    path.join(appRoot, 'src/components/LiveCallScreen.tsx'),
    'utf8',
  )

  assert.match(appSource, /createRippleSignal,[\s\S]*enqueueRippleSignal/)
  assert.match(
    appSource,
    /const \[rippleSignals, setRippleSignals\] = useState<readonly RippleSignal\[\]>\(\[\]\)/,
  )
  assert.equal((appSource.match(/createRippleSignal\(/g) ?? []).length, 3)
  assert.equal((appSource.match(/enqueueRippleSignal\(/g) ?? []).length, 3)
  const toolStart = appSource.indexOf('onToolResult: (event) => {')
  const toolEnd = appSource.indexOf('\n        onAudio:', toolStart)
  const toolCallback = appSource.slice(toolStart, toolEnd)
  assert.ok(toolStart >= 0 && toolEnd > toolStart)
  assert.match(toolCallback, /if \(!ownsSession\(\)\) return/)
  assert.match(toolCallback, /const signal = createRippleSignal\('tool'\)/)
  assert.match(
    toolCallback,
    /setRippleSignals\(\(current\) => enqueueRippleSignal\(current, signal\)\)/,
  )

  const interruptedStart = appSource.indexOf('onInterrupted: () => {')
  const interruptedEnd = appSource.indexOf('\n        onFrameRequested:', interruptedStart)
  const interruptedCallback = appSource.slice(interruptedStart, interruptedEnd)
  assert.ok(interruptedStart >= 0 && interruptedEnd > interruptedStart)
  const clearOutput = interruptedCallback.indexOf('media.clearOutput()')
  const interruptSignal = interruptedCallback.indexOf("createRippleSignal('interrupt')")
  assert.notEqual(clearOutput, -1, 'interrupt handling should clear local playback')
  assert.notEqual(interruptSignal, -1, 'interrupt handling should emit its Ripple signal')
  assert.ok(
    clearOutput < interruptSignal,
    'interrupt Ripple must follow local playback clearing',
  )
  assert.match(
    appSource,
    /setUserText\(''\)\s*setAssistantText\(''\)\s*const signal = createRippleSignal\('speech'\)\s*setRippleSignals\(\(current\) => enqueueRippleSignal\(current, signal\)\)\s*void session\.speechStarted\(\)/,
  )
  assert.doesNotMatch(appSource, /rippleSignalIdRef|nextRippleSignalId/)
  const openCallStart = appSource.indexOf('const openCall = (')
  const openCallEnd = appSource.indexOf('\n  const openConversationMemory', openCallStart)
  const openCallSource = appSource.slice(openCallStart, openCallEnd)
  assert.ok(openCallStart >= 0 && openCallEnd > openCallStart)
  const signalReset = openCallSource.indexOf('setRippleSignals([])')
  assert.notEqual(signalReset, -1, 'a new call should clear the previous signal')
  assert.ok(
    signalReset < openCallSource.indexOf("navigateTo('call')"),
    'a new call must clear the previous opaque signal before mounting the orb',
  )
  const stopCallStart = appSource.indexOf('const stopCall = useCallback')
  const stopCallEnd = appSource.indexOf('\n\n  const leaveCall', stopCallStart)
  const stopCallSource = appSource.slice(stopCallStart, stopCallEnd)
  assert.ok(stopCallStart >= 0 && stopCallEnd > stopCallStart)
  assert.match(
    stopCallSource,
    /setRippleSignals\(\[\]\)/,
    'leaving or unmounting a call should clear queued Ripple signals',
  )
  assert.match(appSource, /consumeRippleSignalsThrough\(current, signalId\)/)
  assert.match(appSource, /rippleSignals=\{rippleSignals\}/)
  assert.match(appSource, /onRippleSignalsConsumed=\{onRippleSignalsConsumed\}/)
  assert.match(callSource, /rippleSignals: readonly RippleSignal\[\]/)
  assert.match(callSource, /rippleSignals=\{rippleSignals\}/)
})

test('mobile live call renders typed result receipts without unsafe HTML', () => {
  const resultPath = path.join(
    appRoot,
    'src/components/LiveResultSheet.tsx',
  )
  assert.equal(existsSync(resultPath), true, 'LiveResultSheet.tsx should exist')

  const resultSource = readFileSync(resultPath, 'utf8')
  const callSource = readFileSync(
    path.join(appRoot, 'src/components/LiveCallScreen.tsx'),
    'utf8',
  )
  const callCssSource = readFileSync(
    path.join(appRoot, 'src/live/LiveCall.css'),
    'utf8',
  )

  assert.match(resultSource, /memory_receipt/)
  assert.match(resultSource, /todo_receipt/)
  assert.match(resultSource, /weather/)
  assert.match(resultSource, /search/)
  assert.match(resultSource, /WarningCircle/)
  assert.match(resultSource, /live-result-icon is-receipt/)
  assert.match(resultSource, /is-failure/)
  assert.doesNotMatch(resultSource, /weight=/)
  for (const icon of ['CheckCircle', 'CloudSun', 'ListChecks', 'MagnifyingGlass', 'WarningCircle', 'X']) {
    assert.match(
      resultSource,
      new RegExp(`<${icon}(?:\\s|\\/|>)`),
      `${icon} should use the shared Lucide outline style`,
    )
  }
  assert.doesNotMatch(resultSource, /<a\b|href=|target=/)
  assert.match(resultSource, /openExternalUrl/)
  assert.doesNotMatch(resultSource, /dangerouslySetInnerHTML/)
  assert.doesNotMatch(resultSource, /JSON\.stringify|<pre\b/)
  assert.match(callSource, /<LiveResultSheet/)
  assert.match(callSource, /className="live-artifact-sheet"/)
  assert.match(callSource, /<AuthenticatedArtifact/)
  assert.match(callSource, /return <img src=\{source\}/)

  const resultStyles = callCssSource.slice(
    callCssSource.indexOf('.live-output-tray'),
    callCssSource.indexOf('@keyframes camera-focus-in'),
  )
  assert.match(resultStyles, /\.live-output-tray\s*\{[^}]*position:\s*relative;/s)
  assert.match(resultStyles, /\.live-output-tray\s*\{[^}]*max-height:\s*min\(34dvh, 300px\);/s)
  assert.match(resultStyles, /\.live-output-tray\s*\{[^}]*margin:\s*0 0 calc\(70px \+ max\(14px, env\(safe-area-inset-bottom\)\)\);/s)
  assert.match(resultStyles, /\.live-output-tray\s*\{[^}]*animation:\s*live-result-enter 280ms/s)
  assert.match(resultStyles, /\.live-result-sheet\s*\{[^}]*overflow-y:\s*auto;/s)
  assert.match(resultStyles, /\.live-result-card\s*\{[^}]*padding:\s*14px 8px 14px 14px;[^}]*border:\s*1px solid var\(--line\);[^}]*border-radius:\s*18px;[^}]*background:\s*rgb\(24 24 24 \/ 94%\);/s)
  assert.doesNotMatch(resultStyles, /rgb\(158 220 255|rgb\(10 23 32/)
  assert.match(resultStyles, /\.live-result-dismiss\s*\{[^}]*width:\s*44px;[^}]*height:\s*44px;[^}]*font-size:\s*18px;/s)
  assert.match(resultStyles, /\.live-result-icon\s*\{[^}]*font-size:\s*20px;/s)
  assert.match(resultStyles, /\.live-result-icon\.is-brand\s*\{[^}]*color:\s*var\(--orb-cobalt\);/s)
  assert.match(resultStyles, /\.live-result-icon\.is-receipt\s*\{[^}]*color:\s*var\(--success\);/s)
  assert.match(resultStyles, /\.live-result-icon\.is-failure\s*\{[^}]*color:\s*var\(--danger\);/s)
  assert.match(resultStyles, /font-size:\s*14px;/)
  assert.match(resultStyles, /font-size:\s*(?:10|11|12)px;/)
  assert.match(callCssSource, /\.live-call-screen\.has-results \.live-stage\s*\{[^}]*min-height:\s*220px;[^}]*transform:\s*none;/s)
  assert.match(callCssSource, /\.live-call-screen\.has-results \.live-orb-canvas,[\s\S]*?\.live-call-screen\.has-results \.live-orb-fallback\s*\{[^}]*animation:\s*none;[^}]*transform:\s*scale\(0\.70\);/s)
  assert.doesNotMatch(callCssSource, /\.live-call-screen\.has-results \.call-controls/)
})

test('reduced motion preserves static result geometry without transitions or keyframes', () => {
  const callCssSource = readFileSync(
    path.join(appRoot, 'src/live/LiveCall.css'),
    'utf8',
  )
  const reducedMotionSource = callCssSource.slice(
    callCssSource.lastIndexOf('@media (prefers-reduced-motion: reduce)'),
  )

  assert.match(
    reducedMotionSource,
    /\.live-call-screen\.has-results \.live-stage\s*\{[^}]*transform:\s*none;[^}]*transition:\s*none;/s,
  )
  assert.match(
    reducedMotionSource,
    /\.live-call-screen\.has-results \.live-orb-canvas,[\s\S]*?\.live-call-screen\.has-results \.live-orb-fallback\s*\{[^}]*animation:\s*none;[^}]*transform:\s*scale\(0\.70\);[^}]*transition:\s*none;/s,
  )
})

test('artifact-only output activates result presentation without moving controls', () => {
  const callSource = readFileSync(
    path.join(appRoot, 'src/components/LiveCallScreen.tsx'),
    'utf8',
  )

  assert.match(
    callSource,
    /const hasOutput = results\.length > 0 \|\| artifacts\.length > 0/,
  )
  assert.match(
    callSource,
    /className=\{`call-screen live-call-screen[\s\S]*\$\{hasOutput \? 'has-results' : ''\}`\}/,
  )
  assert.match(callSource, /\{hasOutput && \(\s*<div className="live-output-tray">/s)
  const controlsStart = callSource.indexOf('<footer className="call-controls">')
  const controlsEnd = callSource.indexOf('</footer>', controlsStart)
  assert.ok(controlsStart >= 0 && controlsEnd > controlsStart)
  assert.doesNotMatch(callSource.slice(controlsStart, controlsEnd), /hasOutput/)
})

test('mobile opens live search sources through the native external-browser capability', () => {
  const packageJson = JSON.parse(
    readFileSync(path.join(appRoot, 'package.json'), 'utf8'),
  )
  const cargo = readFileSync(path.join(appRoot, 'src-tauri/Cargo.toml'), 'utf8')
  const rust = readFileSync(path.join(appRoot, 'src-tauri/src/lib.rs'), 'utf8')
  const capability = JSON.parse(
    readFileSync(path.join(appRoot, 'src-tauri/capabilities/default.json'), 'utf8'),
  )
  const openerCapabilityPath = path.join(
    appRoot,
    'src-tauri/capabilities/external-http-opener.json',
  )
  const externalLinkPath = path.join(appRoot, 'src/live/externalLinks.ts')

  assert.equal(existsSync(externalLinkPath), true, 'externalLinks.ts should exist')
  assert.equal(typeof packageJson.dependencies['@tauri-apps/plugin-opener'], 'string')
  assert.match(
    cargo,
    /\[target\.'cfg\(not\(target_os = "ios"\)\)'\.dependencies\][\s\S]*tauri-plugin-opener/,
  )
  assert.match(
    rust,
    /#\[cfg\(not\(target_os = "ios"\)\)\][\s\S]*tauri_plugin_opener::init\(\)/,
  )
  assert.equal(
    capability.permissions.some((permission) =>
      typeof permission === 'object'
        ? permission.identifier === 'opener:allow-open-url'
        : permission === 'opener:allow-open-url',
    ),
    false,
  )
  assert.equal(existsSync(openerCapabilityPath), true)
  const openerCapability = JSON.parse(
    readFileSync(openerCapabilityPath, 'utf8'),
  )
  assert.deepEqual(openerCapability.platforms, [
    'linux',
    'macOS',
    'windows',
    'android',
  ])
  const openerPermission = openerCapability.permissions.find(
    (permission) =>
      typeof permission === 'object' &&
      permission.identifier === 'opener:allow-open-url',
  )
  assert.deepEqual(openerPermission, {
    identifier: 'opener:allow-open-url',
    allow: [{ url: 'https://*' }, { url: 'http://*' }],
  })
})

test('a failed response preserves confirmed live receipts', () => {
  const appSource = readFileSync(path.join(appRoot, 'src/App.tsx'), 'utf8')
  const responseFailure = appSource.match(
    /onResponseFailed: \(message\) => \{[\s\S]*?\n\s*\},\n\s*onAssistantText:/,
  )?.[0]

  assert.ok(responseFailure, 'onResponseFailed callback should be present')
  assert.doesNotMatch(responseFailure, /dispatchLiveResults/)
})

test('mobile live call controls retain scoped focus and size contracts', () => {
  const cssSource = readFileSync(
    path.join(appRoot, 'src/live/LiveCall.css'),
    'utf8',
  )

  assert.match(
    cssSource,
    /\.live-call-screen \.control-button:focus-visible,\s*\.live-call-screen \.end-button:focus-visible\s*\{[^}]*outline:\s*3px solid #9bc3ff;[^}]*outline-offset:\s*3px;/s,
  )
  assert.match(
    cssSource,
    /\.live-call-screen \.control-button,\s*\.live-call-screen \.end-button\s*\{[^}]*min-width:\s*44px;[^}]*min-height:\s*44px;/s,
  )
  assert.match(
    cssSource,
    /\.live-call-screen \.control-button\.is-active\s*\{[^}]*background:\s*#f5f4f0;[^}]*color:\s*#07080c;/s,
  )
  assert.match(
    cssSource,
    /\.live-call-screen \.end-button\s*\{[^}]*background:\s*#ed687a;/s,
  )
})

test('mobile live orb releases an interrupted response within 160ms', () => {
  const cssSource = readFileSync(
    path.join(appRoot, 'src/live/LiveCall.css'),
    'utf8',
  )
  const orbSource = readFileSync(
    path.join(appRoot, 'src/components/LiveOrb.tsx'),
    'utf8',
  )

  assert.match(
    orbSource,
    /isInterruptionRelease\(previousState, props\.state\)/,
  )
  assert.match(orbSource, /createInterruptionReleaseLatch/)
  assert.match(orbSource, /interruptionReleaseHeld/)
  assert.match(orbSource, /is-interruption-release/)
  assert.match(cssSource, /\.is-interruption-release\s*\{[^}]*transition-duration:\s*160ms;/s)
  assert.doesNotMatch(
    cssSource,
    /\.live-orb-canvas\.is-listening,\s*\.live-orb-fallback\.is-listening\s*\{[^}]*transition-duration:/s,
  )
})

test('mobile libraries expose accessible shared management controls', () => {
  const appSource = readFileSync(path.join(appRoot, 'src/App.tsx'), 'utf8')
  const cssSource = readFileSync(path.join(appRoot, 'src/App.css'), 'utf8')
  const toolbarSource = readFileSync(
    path.join(appRoot, 'src/components/LibraryToolbar.tsx'),
    'utf8',
  )

  for (const file of [
    'LibraryToolbar.tsx',
    'LibrarySection.tsx',
    'LibraryActions.tsx',
  ]) {
    assert.equal(
      existsSync(path.join(appRoot, 'src/components', file)),
      true,
      `${file} should exist`,
    )
  }

  assert.match(appSource, /aria-label="搜索聊天历史"/)
  assert.match(appSource, /aria-label="搜索视觉记忆"/)
  assert.doesNotMatch(appSource, /对话资料库|视觉资料库|className="history-heading"/)
  assert.match(appSource, /className="library-region"/)
  assert.match(toolbarSource, /className="visually-hidden"/)
  assert.match(appSource, /删除后无法恢复/)
  assert.match(appSource, /groupLibraryItems/)
  assert.match(appSource, /setDebouncedHistoryQuery\(historyQuery\), 250/)
  assert.match(appSource, /setDebouncedMemoryQuery\(memoryQuery\), 250/)
  assert.match(appSource, /closest\('\.library-item-actions, input, textarea'\)/)
  assert.match(appSource, /没有找到相关对话/)
  assert.match(appSource, /已归档的对话会保留，但不会出现在最近记录中/)
  assert.doesNotMatch(appSource, /historyItems\.map\(\(item\)/)
  assert.match(appSource, /memory-library-grid/)
  assert.match(appSource, /没有找到相关记忆/)
  assert.match(
    cssSource,
    /grid-template-columns:\s*repeat\(2, minmax\(0, 1fr\)\)/,
  )
  assert.match(cssSource, /@media \(prefers-reduced-motion: reduce\)/)
  assert.match(cssSource, /\.library-region\s*{[^}]*margin-top:\s*24px/s)
  assert.match(cssSource, /\.history-list \.history-row\s*{[^}]*min-height:\s*64px/s)
  assert.match(cssSource, /\.memory-library-grid\s*{[^}]*gap:\s*8px/s)
})

test('supporting screens use truthful content and the shared warm hierarchy', () => {
  const appSource = readFileSync(path.join(appRoot, 'src/App.tsx'), 'utf8')
  const cssSource = readFileSync(path.join(appRoot, 'src/App.css'), 'utf8')
  const toolbarSource = readFileSync(
    path.join(appRoot, 'src/components/LibraryToolbar.tsx'),
    'utf8',
  )

  assert.match(toolbarSource, /const memoryScopes[\s\S]*?label:\s*'全部'[\s\S]*?label:\s*'图片'/)
  assert.doesNotMatch(toolbarSource, /const memoryScopes[\s\S]*?label:\s*'置顶'/)
  assert.match(toolbarSource, /aria-label=\{`更多\$\{kind\}操作`\}/)
  assert.match(toolbarSource, /scope === 'archived' \? 'all' : 'archived'/)
  assert.match(appSource, /hasCover:\s*Boolean\(item\.cover\)/)
  assert.match(appSource, /className="memory-card-note"/)
  assert.match(
    cssSource,
    /\.memory-card-hit \.memory-card-body strong\s*{[^}]*font-size:\s*14px;/s,
  )
  assert.match(
    cssSource,
    /\.memory-card-hit \.memory-card-body time\s*{[^}]*font-size:\s*10px;/s,
  )

  assert.match(appSource, /className=\{`todo-card todo-card-surface \$\{todoView === 'completed' \? 'is-completed' : ''\}`\}/)
  assert.match(appSource, /className="todo-copy"/)
  assert.doesNotMatch(appSource, /className="todo-row-meta"|className="todo-edit"/)
  assert.match(
    cssSource,
    /\.todo-swipe-shell\s*{[^}]*border:\s*0;[^}]*border-bottom:\s*1px solid var\(--line\);[^}]*border-radius:\s*0;/s,
  )
  assert.match(
    cssSource,
    /\.todo-complete\s*{[^}]*width:\s*44px;[^}]*height:\s*44px;[^}]*border-radius:\s*50%;/s,
  )
  assert.match(cssSource, /\.todo-card\.is-completed\s*{[^}]*opacity:\s*0\.52;/s)
  assert.match(cssSource, /\.todo-card\.is-completed strong\s*{[^}]*text-decoration:\s*line-through;/s)

  for (const copy of [
    '系统状态',
    '通知权限',
    '实时字幕',
    '视觉记忆',
    '麦克风与相机',
    '通话时自动显示你和 Ripple 正在说的内容',
  ]) {
    assert.match(appSource, new RegExp(copy))
  }
  assert.match(appSource, /notificationPermissionLabel\(\)/)
  assert.match(appSource, /navigateTo\('memories'\)/)
  assert.match(appSource, /className="profile-identity"/)
  assert.doesNotMatch(appSource, /<dt>连接服务<\/dt>|<dd>\{server\}<\/dd>/)
  assert.doesNotMatch(appSource, /type="checkbox"|role="switch"/)

  assert.match(
    cssSource,
    /\.memory-detail-sheet,\s*\.todo-editor,\s*\.confirm-dialog\s*{[^}]*border:\s*1px solid var\(--line\);[^}]*background:\s*var\(--surface-raised\);/s,
  )
  assert.match(cssSource, /\.confirm-dialog button\.danger-action\s*{[^}]*color:\s*var\(--danger\);/s)
  assert.match(appSource, /disabled=\{!todoEditor\.title\.trim\(\)\}/)
  assert.match(appSource, /disabled=\{!memoryDraft\.trim\(\)\}/)
  assert.match(appSource, /deleteRequest\.kind === 'todo' \? '这条待办'/)
})

test('history and conversation detail use a compact voice-first hierarchy', () => {
  const appSource = readFileSync(path.join(appRoot, 'src/App.tsx'), 'utf8')
  const cssSource = readFileSync(path.join(appRoot, 'src/App.css'), 'utf8')
  const navigationSource = readFileSync(
    path.join(appRoot, 'src/components/AppNavigation.css'),
    'utf8',
  )
  const toolbarSource = readFileSync(
    path.join(appRoot, 'src/components/LibraryToolbar.tsx'),
    'utf8',
  )

  assert.match(appSource, /className="screen-header history-page-header library-sticky-header"/)
  assert.match(appSource, /className="history-screen history-library-screen"/)
  assert.doesNotMatch(appSource, /history-search-button|document\.getElementById\('history-search'\)/)
  assert.match(toolbarSource, /className="library-search-affordance"/)
  assert.match(toolbarSource, /aria-label=\{`更多\$\{kind\}操作`\}/)
  assert.match(
    cssSource,
    /\.history-page-header h1\s*{[^}]*font-size:\s*23px;/s,
  )
  assert.match(
    cssSource,
    /\.library-search input\s*{[^}]*min-height:\s*44px;[^}]*border-radius:\s*15px;[^}]*font-size:\s*14px;/s,
  )
  assert.match(
    cssSource,
    /\.history-library-screen \.library-region\s*{[^}]*margin-top:\s*16px;/s,
  )
  assert.match(
    cssSource,
    /\.library-toolbar\.is-history \.library-search input\s*{[^}]*border-radius:\s*12px;[^}]*font-size:\s*13px;/s,
  )
  assert.match(
    cssSource,
    /\.library-sticky-header,\s*\.history-library-screen \.library-sticky-header\s*{[^}]*min-height:\s*60px;[^}]*padding-top:\s*max\(6px, env\(safe-area-inset-top\)\);[^}]*padding-bottom:\s*0;/s,
  )
  assert.match(
    cssSource,
    /\.library-query-row\s*{[^}]*grid-template-columns:\s*minmax\(0, 1fr\) 48px;[^}]*align-items:\s*center;/s,
  )
  assert.match(
    cssSource,
    /\.history-library-screen \.history-page-header h1,\s*\.memory-screen \.history-page-header h1,\s*\.todo-screen \.todo-heading h1,\s*\.todo-screen \.screen-header\.library-sticky-header \.todo-heading h1,\s*\.profile-screen > \.screen-header h1\s*{[^}]*font-size:\s*20px;/s,
  )
  assert.match(
    cssSource,
    /\.history-list \.history-row\s*{[^}]*min-height:\s*64px;[^}]*padding:\s*10px 12px;/s,
  )
  assert.match(
    cssSource,
    /\.history-list \.library-item-surface\s*{[^}]*background:\s*var\(--surface-raised\);/s,
    'history rows must cover their off-canvas actions until the row is revealed',
  )
  assert.match(
    cssSource,
    /\.library-row-preview\s*{[^}]*color:\s*var\(--text-secondary\);[^}]*-webkit-line-clamp:\s*1;/s,
  )
  assert.doesNotMatch(appSource, /className="history-voice-fab"/)
  assert.doesNotMatch(navigationSource, /\.history-voice-fab\s*\{/)
  assert.match(appSource, /item\.preview\.trim\(\)/)
  assert.match(
    cssSource,
    /\.message-history article\.is-assistant\s*{[^}]*border:\s*0;[^}]*background:\s*transparent;/s,
  )
  assert.match(
    cssSource,
    /\.message-history article\.is-user\s*{[^}]*border:\s*0;[^}]*border-radius:\s*18px;[^}]*background:\s*var\(--surface\);/s,
  )
  assert.match(appSource, /className="conversation-continuation-bar"/)
  assert.match(appSource, /className="conversation-header-title"/)
  assert.doesNotMatch(appSource, /<div className="conversation-title">/)
  assert.match(
    appSource,
    /className="conversation-continuation-bar"[\s\S]*?onClick=\{\(\) => openCall\('audio', selectedConversation\.id\)\}/,
  )
  assert.match(appSource, /if \(!hasConversationContent\(messages\)\)/)
  assert.match(
    cssSource,
    /\.conversation-actions button\s*{[^}]*border-radius:\s*10px;[^}]*font-size:\s*11px;/s,
  )
  const continuationStart = appSource.indexOf(
    'className="conversation-continuation-bar"',
  )
  const continuationEnd = appSource.indexOf('</aside>', continuationStart)
  const continuationSource = appSource.slice(continuationStart, continuationEnd)
  assert.ok(continuationStart >= 0 && continuationEnd > continuationStart)
  assert.doesNotMatch(continuationSource, /<input|<textarea|Paperclip|attachment/i)
  assert.doesNotMatch(continuationSource, /continuation-voice|AudioWaveform/)
  assert.equal(
    continuationSource.match(/openCall\('audio', selectedConversation\.id\)/g)?.length,
    1,
  )
})
