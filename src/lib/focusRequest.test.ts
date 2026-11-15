import { describe, expect, it } from 'vitest'

import { terminalFocusWasJustRequested } from './focusRequest'

describe('terminalFocusWasJustRequested', () => {
  it('is true for a request made a moment ago', () => {
    expect(terminalFocusWasJustRequested({ ts: 10_000 }, 10_000)).toBe(true)
    expect(terminalFocusWasJustRequested({ ts: 10_000 }, 11_000)).toBe(true)
  })

  it('is false when nothing was requested or the request is old', () => {
    expect(terminalFocusWasJustRequested(null, 10_000)).toBe(false)
    expect(terminalFocusWasJustRequested({ ts: 10_000 }, 12_000)).toBe(false)
  })

  it('ignores a request stamped in the future', () => {
    expect(terminalFocusWasJustRequested({ ts: 20_000 }, 10_000)).toBe(false)
  })
})
