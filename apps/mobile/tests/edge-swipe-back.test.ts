import assert from 'node:assert/strict'
import test from 'node:test'
import {
  EDGE_SWIPE_COMMIT_DISTANCE,
  EDGE_SWIPE_START_ZONE,
  canStartEdgeSwipe,
  edgeSwipeOffset,
  isHorizontalBackIntent,
  shouldCommitEdgeSwipe,
} from '../src/edgeSwipeBack'

test('edge swipe starts only from the left edge with the primary pointer', () => {
  assert.equal(canStartEdgeSwipe(0, 0), true)
  assert.equal(canStartEdgeSwipe(EDGE_SWIPE_START_ZONE, 0), true)
  assert.equal(canStartEdgeSwipe(EDGE_SWIPE_START_ZONE + 1, 0), false)
  assert.equal(canStartEdgeSwipe(12, 2), false)
})

test('edge swipe locks to a deliberate rightward horizontal gesture', () => {
  assert.equal(isHorizontalBackIntent(18, 3), true)
  assert.equal(isHorizontalBackIntent(8, 1), false)
  assert.equal(isHorizontalBackIntent(18, 20), false)
  assert.equal(isHorizontalBackIntent(-20, 0), false)
})

test('edge swipe visual travel is damped and commits at the product threshold', () => {
  assert.equal(edgeSwipeOffset(-10), 0)
  assert.equal(edgeSwipeOffset(1000), 118)
  assert.equal(shouldCommitEdgeSwipe(EDGE_SWIPE_COMMIT_DISTANCE - 1), false)
  assert.equal(shouldCommitEdgeSwipe(EDGE_SWIPE_COMMIT_DISTANCE), true)
})
