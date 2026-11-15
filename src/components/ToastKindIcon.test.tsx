import { render } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { toastKindAccent } from '../lib/toastKind'
import { ToastKindIcon } from './ToastKindIcon'

describe('the neutral icon of a notification', () => {
  it('uses a status colour for a success and an error, and the plain accent for progress', () => {
    expect(toastKindAccent('success')).toBe('var(--status-working)')
    expect(toastKindAccent('error')).toBe('var(--status-offline)')
    expect(toastKindAccent('progress')).toBe('var(--accent)')
  })

  it('draws a different icon for each kind', () => {
    const shapes = (['progress', 'success', 'error'] as const).map(
      (kind) => render(<ToastKindIcon kind={kind} />).container.innerHTML,
    )
    expect(new Set(shapes).size).toBe(3)
    for (const shape of shapes) expect(shape).toContain('<svg')
  })
})
