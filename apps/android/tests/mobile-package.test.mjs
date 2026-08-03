import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'

const appRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const sourceHash = '37eac2542f29286d0793b52a0b3fb701f89fee4a1d1cdd56bb806765073824e1'

function readPngSize(file) {
  const bytes = readFileSync(file)
  assert.equal(bytes.toString('ascii', 1, 4), 'PNG', `${file} must be a PNG`)
  return {
    width: bytes.readUInt32BE(16),
    height: bytes.readUInt32BE(20),
  }
}

function expectSquare(file, size) {
  assert.deepEqual(readPngSize(file), { width: size, height: size }, file)
}

test('mobile package has the supplied icon and iOS media permissions', () => {
  const source = path.join(appRoot, 'src-tauri/icons/ripple-live-source.png')
  const sourceBytes = readFileSync(source)
  assert.equal(createHash('sha256').update(sourceBytes).digest('hex'), sourceHash)
  assert.deepEqual(readPngSize(source), { width: 254, height: 256 })

  expectSquare(path.join(appRoot, 'src-tauri/icons/icon.png'), 512)
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

  const plist = readFileSync(
    path.join(appRoot, 'src-tauri/gen/apple/ripple-live_iOS/Info.plist'),
    'utf8',
  )
  assert.match(plist, /<key>NSCameraUsageDescription<\/key>/)
  assert.match(plist, /<key>NSMicrophoneUsageDescription<\/key>/)
})
