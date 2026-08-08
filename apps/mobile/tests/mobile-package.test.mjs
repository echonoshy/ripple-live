import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { existsSync, readFileSync } from 'node:fs'
import path from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'

const appRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const sourceHash = 'f52273c5c3e08ff85e5afd8ae5de8ab2014951fd03619d769b42537db1885872'

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
  assert.deepEqual({ width, height }, { width: 1206, height: 1206 })
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

test('mobile uses protocol v4 semantic endpointing', () => {
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
  assert.match(appSource, /onInterrupted: \(\) => media\.clearOutput\(\)/)
  assert.match(protocolSource, /REALTIME_PROTOCOL_VERSION = 4/)
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
  assert.match(appSource, /\n\s*已完成\n\s*<\/button>/)
  assert.match(appSource, /完成后会归档在“已完成”中/)
  assert.match(appSource, /完成：\$\{formatHistoryTime\(todo\.completed_at\)\}/)
  assert.match(cssSource, /\.todo-view-switch/)
})

test('mobile home presents video as the primary call entry', () => {
  const appSource = readFileSync(path.join(appRoot, 'src/App.tsx'), 'utf8')
  const cssSource = readFileSync(path.join(appRoot, 'src/App.css'), 'utf8')

  assert.match(appSource, /<h1>打开镜头，开始聊聊<\/h1>/)
  assert.match(appSource, /<strong>开始视频通话<\/strong>/)
  assert.match(appSource, /<small>让我看见现场<\/small>/)
  assert.match(appSource, /aria-label="开始语音通话"/)
  assert.equal(
    (appSource.match(/className="launch-button call-entry"/g) ?? []).length,
    0,
  )
  assert.match(appSource, /className="launch-button call-entry is-video"/)
  assert.match(appSource, /className="launch-button call-entry is-voice"/)
  assert.match(
    cssSource,
    /\.launch-actions\s*{[^}]*grid-template-columns:\s*minmax\(0, 1fr\) 68px/s,
  )
  assert.match(cssSource, /animation:\s*ripple-ready-pulse 3\.5s/)
  assert.match(cssSource, /@media \(prefers-reduced-motion: reduce\)/)
  for (const color of [
    '#0f0d12',
    '#19161d',
    '#24202a',
    '#302b36',
    '#f5f2f7',
    '#9046ff',
  ]) {
    assert.match(cssSource, new RegExp(color))
  }
  assert.match(cssSource, /@media \(max-width: 359px\)/)
  assert.match(cssSource, /\.launch-button\s*{[^}]*min-width:\s*44px/s)
  assert.match(cssSource, /--voice-accent:\s*#b98aff/)
  assert.match(cssSource, /--video-accent:\s*#e8ddff/)
  assert.match(cssSource, /\.call-entry\.is-voice/)
  assert.match(cssSource, /\.call-entry\.is-video/)
})

test('mobile live orb uses a single canvas renderer with a static fallback', () => {
  for (const file of [
    'live/orbRenderer.ts',
    'components/LiveOrb.tsx',
    'live/LiveCall.css',
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
  assert.doesNotMatch(orbSource, /lottie|video/i)
})

test('mobile live call uses the immersive presentation contract', () => {
  const appSource = readFileSync(path.join(appRoot, 'src/App.tsx'), 'utf8')
  const callSource = readFileSync(
    path.join(appRoot, 'src/components/LiveCallScreen.tsx'),
    'utf8',
  )

  assert.match(callSource, /<LiveOrb/)
  assert.match(callSource, /<LiveCaption/)
  assert.doesNotMatch(callSource, /HandPalm|打断回答/)
  assert.match(callSource, /aria-label=\{muted \? '取消静音' : '静音'\}/)
  assert.match(callSource, /aria-label="结束通话"/)
  assert.match(callSource, /<small aria-hidden="true">\{formatDuration\(elapsed\)\}<\/small>/)
  assert.match(
    appSource,
    /setUserText\(''\)\s*setAssistantText\(''\)\s*void session\.speechStarted\(\)/,
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
  assert.match(cssSource, /\.history-list \.history-row\s*{[^}]*min-height:\s*74px/s)
  assert.match(cssSource, /\.memory-library-grid\s*{[^}]*gap:\s*8px/s)
})
