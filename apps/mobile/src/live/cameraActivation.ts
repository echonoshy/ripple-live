import type { SessionState } from '../realtime/RealtimeSession'

type ActivationCommit = { cameraRequested: boolean }

function activationAllowed(state: SessionState) {
  return state === 'listening' ||
    state === 'thinking' ||
    state === 'using_tool' ||
    state === 'speaking'
}

/** Owns the async media-start boundary for exactly one realtime session. */
export function createCameraActivationGuard(initialCameraIntent: boolean) {
  let generation = 0
  let state: SessionState = 'idle'
  let pending: number | null = null
  let cameraIntent = initialCameraIntent

  const invalidate = () => {
    generation += 1
    pending = null
    cameraIntent = false
  }

  return {
    begin() {
      if (!activationAllowed(state) || pending !== null) return null
      generation += 1
      pending = generation
      return pending
    },
    commit(token: number): ActivationCommit | null {
      if (
        pending !== token ||
        generation !== token ||
        !activationAllowed(state)
      ) return null
      pending = null
      const cameraRequested = cameraIntent
      cameraIntent = false
      return { cameraRequested }
    },
    invalidate,
    transition(nextState: SessionState) {
      state = nextState
      if (
        nextState === 'ended' ||
        nextState === 'error' ||
        (pending !== null && !activationAllowed(nextState))
      ) invalidate()
    },
  }
}
