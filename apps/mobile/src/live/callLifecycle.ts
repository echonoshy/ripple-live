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
