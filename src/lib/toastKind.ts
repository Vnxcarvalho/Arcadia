import type { ToastKind } from '../stores/uiStore'

/** Accent colour of a notification that reports progress, a success or an error. */
export function toastKindAccent(kind: ToastKind): string {
  if (kind === 'success') return 'var(--status-working)'
  if (kind === 'error') return 'var(--status-offline)'
  return 'var(--accent)'
}
