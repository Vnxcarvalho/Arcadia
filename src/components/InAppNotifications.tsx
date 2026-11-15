import { Bell, X } from 'lucide-react'
import { type CSSProperties, useEffect } from 'react'

import { agentAccentVar } from '../lib/agentProviders'
import { useT } from '../lib/i18n'
import { toastKindAccent } from '../lib/toastKind'
import { useProjectsStore } from '../stores/projectsStore'
import { type InAppToast, useUiStore } from '../stores/uiStore'
import { AgentIcon } from './icons/AgentIcons'
import styles from './InAppNotifications.module.css'
import { ToastKindIcon } from './ToastKindIcon'

function ToastItem({ toast }: { toast: InAppToast }) {
  const t = useT()
  const dismissToast = useUiStore((s) => s.dismissToast)
  const uiTheme = useProjectsStore((s) => s.preferences.uiTheme)

  useEffect(() => {
    // A toast that asks something has to outlive a glance, or the offer is gone before it is read.
    const timer = window.setTimeout(
      () => dismissToast(toast.id),
      toast.actions?.length ? 20000 : 6500,
    )
    return () => window.clearTimeout(timer)
  }, [dismissToast, toast.id, toast.actions])

  const accentStyle = {
    '--toast-accent': toast.agent
      ? agentAccentVar(toast.agent)
      : toastKindAccent(toast.kind ?? 'progress'),
  } as CSSProperties

  return (
    <div className={styles.toast} role="status" style={accentStyle}>
      <div className={styles.toastIcon} aria-hidden>
        {toast.agent ? (
          <AgentIcon type={toast.agent} size={16} theme={uiTheme} />
        ) : toast.kind ? (
          <ToastKindIcon kind={toast.kind} />
        ) : (
          <Bell size={14} />
        )}
      </div>
      <div className={styles.toastText}>
        <strong>{toast.title}</strong>
        <span title={toast.body}>{toast.body}</span>
        {toast.actions?.length ? (
          <div className={styles.toastActions}>
            {toast.actions.map((action, index) => (
              <button
                key={action.label}
                type="button"
                className={
                  action.quiet
                    ? styles.toastActionQuiet
                    : index === 0
                      ? styles.toastAction
                      : styles.toastActionSecondary
                }
                onClick={() => {
                  action.run()
                  dismissToast(toast.id)
                }}
              >
                {action.label}
              </button>
            ))}
          </div>
        ) : null}
      </div>
      <button
        type="button"
        className={styles.toastClose}
        onClick={() => dismissToast(toast.id)}
        aria-label={t('common.close')}
        title={t('common.close')}
      >
        <X size={14} />
      </button>
    </div>
  )
}

export function InAppNotifications() {
  const toasts = useUiStore((s) => s.toasts)
  if (toasts.length === 0) return null

  return (
    <div className={styles.toastStack} aria-live="polite" aria-relevant="additions">
      {toasts.map((toast) => (
        <ToastItem key={toast.id} toast={toast} />
      ))}
    </div>
  )
}
