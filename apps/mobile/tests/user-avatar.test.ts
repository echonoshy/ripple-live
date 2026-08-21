import assert from 'node:assert/strict'
import test from 'node:test'
import { avatarInitial, cropSourceRect } from '../src/avatar'

test('avatar fallback uses the first uppercase email character', () => {
  assert.equal(avatarInitial('lake@example.com'), 'L')
  assert.equal(avatarInitial('  ripple@example.com'), 'R')
  assert.equal(avatarInitial(''), 'R')
})

test('avatar crop centers landscape and portrait images', () => {
  const landscape = cropSourceRect({ width: 1000, height: 500 }, 1, { x: 0, y: 0 })
  const portrait = cropSourceRect({ width: 500, height: 1000 }, 1, { x: 0, y: 0 })
  assert.ok(Math.abs(landscape.x - 250) < 0.001)
  assert.equal(landscape.y, 0)
  assert.ok(Math.abs(landscape.side - 500) < 0.001)
  assert.equal(portrait.x, 0)
  assert.ok(Math.abs(portrait.y - 250) < 0.001)
  assert.ok(Math.abs(portrait.side - 500) < 0.001)
})

test('avatar crop accounts for zoom and drag offset', () => {
  const crop = cropSourceRect({ width: 1000, height: 500 }, 2, { x: 56, y: 0 })
  assert.ok(Math.abs(crop.x - 325) < 0.001)
  assert.ok(Math.abs(crop.y - 125) < 0.001)
  assert.ok(Math.abs(crop.side - 250) < 0.001)
})
