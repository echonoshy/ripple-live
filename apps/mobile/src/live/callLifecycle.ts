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

function validConversationId(value: unknown): value is string {
  return typeof value === 'string' && value.trim().length > 0
}

export function createConversationOwnership() {
  let generation = 0
  let conversationId: string | null = null

  const snapshot = () => ({ owner: generation, conversationId })

  return {
    begin(nextConversationId?: string) {
      generation += 1
      conversationId = validConversationId(nextConversationId)
        ? nextConversationId
        : null
      return snapshot()
    },
    current: snapshot,
    confirm(owner: number, nextConversationId: string) {
      if (owner !== generation || !validConversationId(nextConversationId)) {
        return null
      }
      conversationId = nextConversationId
      return conversationId
    },
    release(owner: number) {
      if (owner !== generation) return false
      generation += 1
      conversationId = null
      return true
    },
    invalidate() {
      generation += 1
      conversationId = null
    },
  }
}

export function createLatestNavigationGuard() {
  let generation = 0

  return {
    begin() {
      generation += 1
      return generation
    },
    owns(owner: number) {
      return owner === generation
    },
    invalidate() {
      generation += 1
    },
  }
}
