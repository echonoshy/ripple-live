export function createSingleFlight(operation: () => Promise<void>) {
  let active: Promise<void> | null = null

  return () => {
    if (active) return active

    const request = operation()
    active = request
    const clear = () => {
      if (active === request) active = null
    }
    void request.then(clear, clear)
    return request
  }
}

type CallLifecyclePhase =
  | 'idle'
  | 'opening'
  | 'active'
  | 'leaving'
  | 'failed'

export function createCallLifecycleGuard() {
  let generation = 0
  let phase: CallLifecyclePhase = 'idle'

  return {
    requestOpen() {
      if (phase === 'opening' || phase === 'active' || phase === 'leaving') {
        return false
      }
      generation += 1
      phase = 'opening'
      return true
    },
    claimStart() {
      if (phase !== 'opening') return null
      phase = 'active'
      return generation
    },
    owns(owner: number) {
      return phase === 'active' && owner === generation
    },
    fail(owner: number) {
      if (phase !== 'active' || owner !== generation) return false
      generation += 1
      phase = 'failed'
      return true
    },
    beginLeave() {
      if (phase === 'leaving') return false
      generation += 1
      phase = 'leaving'
      return true
    },
    finishLeave() {
      if (phase === 'leaving') phase = 'idle'
    },
    canAutoStart() {
      return phase === 'opening'
    },
    invalidate() {
      generation += 1
      phase = 'idle'
    },
  }
}
