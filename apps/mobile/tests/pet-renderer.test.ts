import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import { fileURLToPath } from 'node:url'
import {
  PET_ANIMATIONS,
  PET_ATLAS,
  PET_HD_ATLAS,
  PET_HD_ROWS,
  PET_GIF_STATES,
  petFrameAt,
} from '../src/live/petRenderer.ts'

test('starry avatar asset is the packaged v2 webp atlas', () => {
  const asset = fileURLToPath(
    new URL('../src/assets/starry-avatar.webp', import.meta.url),
  )
  const bytes = readFileSync(asset)
  assert.equal(bytes.toString('ascii', 0, 4), 'RIFF')
  assert.equal(bytes.toString('ascii', 8, 12), 'WEBP')
  assert.deepEqual(PET_ATLAS, {
    width: 1536,
    height: 2288,
    columns: 8,
    rows: 11,
    cellWidth: 192,
    cellHeight: 208,
  })
})

test('animated GIF assets cover every rendered live state', () => {
  for (const state of [
    'idle',
    'waving',
    'failed',
    'waiting',
    'running',
    'review',
  ]) {
    const asset = fileURLToPath(
      new URL(`../src/assets/pet-gifs/starry-avatar-${state}.gif`, import.meta.url),
    )
    const bytes = readFileSync(asset)
    assert.equal(bytes.toString('ascii', 0, 3), 'GIF')
    assert.equal(bytes.readUInt16LE(6), 384)
    assert.equal(bytes.readUInt16LE(8), 416)
  }
  assert.deepEqual(PET_GIF_STATES, {
    idle: 'idle',
    connecting: 'waiting',
    listening: 'waiting',
    thinking: 'running',
    tool: 'review',
    speaking: 'idle',
    ended: 'waving',
    error: 'failed',
  })
})

test('retina state atlas packages every live state at two times cell resolution', () => {
  const asset = fileURLToPath(
    new URL('../src/assets/starry-avatar-states@2x.png', import.meta.url),
  )
  const bytes = readFileSync(asset)
  assert.equal(bytes.toString('ascii', 1, 4), 'PNG')
  assert.equal(bytes.readUInt32BE(16), 3072)
  assert.equal(bytes.readUInt32BE(20), 2496)
  assert.deepEqual(PET_HD_ATLAS, {
    width: 3072,
    height: 2496,
    columns: 8,
    rows: 6,
    cellWidth: 384,
    cellHeight: 416,
  })
  assert.deepEqual(PET_HD_ROWS, {
    idle: 0,
    connecting: 3,
    listening: 3,
    thinking: 4,
    tool: 5,
    speaking: 0,
    ended: 1,
    error: 2,
  })
})

test('live states use only validated standard pet rows and frame counts', () => {
  assert.deepEqual(
    Object.fromEntries(
      Object.entries(PET_ANIMATIONS).map(([state, animation]) => [
        state,
        [animation.row, animation.durations.length],
      ]),
    ),
    {
      idle: [0, 6],
      connecting: [6, 6],
      listening: [6, 6],
      thinking: [7, 6],
      tool: [8, 6],
      speaking: [0, 6],
      ended: [3, 4],
      error: [5, 8],
    },
  )
})

test('reduced motion freezes every state on its first frame', () => {
  for (const state of Object.keys(PET_ANIMATIONS)) {
    assert.deepEqual(
      petFrameAt(state as keyof typeof PET_ANIMATIONS, 50_000, true),
      { row: PET_ANIMATIONS[state as keyof typeof PET_ANIMATIONS].row, column: 0 },
    )
  }
})

test('idle loops with its authored non-uniform frame timing', () => {
  assert.deepEqual(petFrameAt('idle', 279, false), { row: 0, column: 0 })
  assert.deepEqual(petFrameAt('idle', 280, false), { row: 0, column: 1 })
  assert.deepEqual(petFrameAt('idle', 1100, false), { row: 0, column: 0 })
})

test('one-shot completion and error animations hold their final frame', () => {
  assert.deepEqual(petFrameAt('ended', 10_000, false), { row: 3, column: 3 })
  assert.deepEqual(petFrameAt('error', 10_000, false), { row: 5, column: 7 })
})
