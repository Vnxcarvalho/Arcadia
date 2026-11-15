import { CheckCircle2, Download, TriangleAlert } from 'lucide-react'

import type { ToastKind } from '../stores/uiStore'

/** The neutral icon of a notification that reports progress, a success or an error. */
export function ToastKindIcon({ kind, size = 14 }: { kind: ToastKind; size?: number }) {
  if (kind === 'success') return <CheckCircle2 size={size} />
  if (kind === 'error') return <TriangleAlert size={size} />
  return <Download size={size} />
}
