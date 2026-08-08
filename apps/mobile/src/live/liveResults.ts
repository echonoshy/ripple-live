import type { LiveResult } from '../realtime/toolResults'

const MAX_LIVE_RESULTS = 3

export type LiveResultsAction =
  | { type: 'add'; result: LiveResult }
  | { type: 'dismiss'; callId: string }
  | { type: 'clear' }

export function liveResultsReducer(
  results: LiveResult[],
  action: LiveResultsAction,
): LiveResult[] {
  switch (action.type) {
    case 'add': {
      const duplicateIndex = results.findIndex(
        (result) => result.callId === action.result.callId,
      )
      if (duplicateIndex >= 0) {
        return results.map((result, index) =>
          index === duplicateIndex ? action.result : result,
        )
      }
      return [...results, action.result].slice(-MAX_LIVE_RESULTS)
    }
    case 'dismiss':
      return results.filter((result) => result.callId !== action.callId)
    case 'clear':
      return []
  }
}
