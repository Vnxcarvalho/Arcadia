import * as Dialog from '@radix-ui/react-dialog'
import { X } from 'lucide-react'
import { type ReactNode, useRef } from 'react'

import { terminalFocusWasJustRequested } from '../../lib/focusRequest'
import { useT } from '../../lib/i18n'
import { useUiStore } from '../../stores/uiStore'
import styles from './Modal.module.css'

type Props = {
  open: boolean
  onClose: () => void
  title: ReactNode
  children: ReactNode
  footer?: ReactNode
  width?: number
  /** Set when this modal is opened from inside another one, so it layers above it. */
  nested?: boolean
  className?: string
  /** Optional control rendered in the header between the title and the close button. */
  headerAction?: ReactNode
}

export function Modal({
  open,
  onClose,
  title,
  children,
  footer,
  width = 440,
  nested = false,
  className,
  headerAction,
}: Props) {
  const t = useT()
  // Read when the window closes, which is after the terminal it opened asked for the keyboard.
  const focusRequest = useUiStore((s) => s.focusRequest)
  const focusRequestRef = useRef(focusRequest)
  focusRequestRef.current = focusRequest
  return (
    <Dialog.Root open={open} onOpenChange={(v) => !v && open && onClose()}>
      <Dialog.Portal>
        <Dialog.Overlay className={`${styles.overlay} ${nested ? styles.overlayNested : ''}`} />
        <Dialog.Content
          data-alethe-modal-content=""
          className={`${styles.content} ${nested ? styles.contentNested : ''} ${className ?? ''}`}
          style={{ width }}
          aria-describedby={undefined}
          onInteractOutside={(event) => {
            const target = event.target as Element | null
            if (target?.closest('[data-alethe-dropdown-menu]')) event.preventDefault()
          }}
          onEscapeKeyDown={(event) => {
            if (document.querySelector('[data-alethe-dropdown-menu]')) event.preventDefault()
          }}
          onCloseAutoFocus={(event) => {
            // A terminal opened by this window asked for the keyboard; giving it back to the button that
            // opened the window would leave the first keys, such as Tab, going nowhere.
            if (terminalFocusWasJustRequested(focusRequestRef.current ?? null)) {
              event.preventDefault()
            }
          }}
          onOpenAutoFocus={(e) => {
            const root = e.currentTarget as HTMLElement | null
            const input = root?.querySelector<HTMLElement>('input,textarea,[data-autofocus]')
            if (input) {
              e.preventDefault()
              input.focus()
            }
          }}
        >
          <header className={styles.header}>
            <Dialog.Title className={styles.title}>{title}</Dialog.Title>
            <div className={styles.headerActions}>
              {headerAction}
              <Dialog.Close asChild>
                <button type="button" aria-label={t('common.close')} className={styles.close}>
                  <X size={16} />
                </button>
              </Dialog.Close>
            </div>
          </header>
          <div className={styles.body}>{children}</div>
          {footer ? <footer className={styles.footer}>{footer}</footer> : null}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  )
}
