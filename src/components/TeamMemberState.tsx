import { useEffect, useState } from 'react'

import { useT } from '../lib/i18n'
import { type MemberState, memberState } from '../lib/teamView'
import { useTerminalsStore } from '../stores/terminalsStore'
import styles from './TeamMemberState.module.css'

const POLL_MS = 800

/** What a team member is doing, read from how long ago its terminal last printed something. */
export function TeamMemberState({ ptyId }: { ptyId: string | null }) {
  const t = useT()
  const [state, setState] = useState<MemberState>('ready')

  useEffect(() => {
    const read = () =>
      setState(
        memberState(ptyId ? useTerminalsStore.getState().byPtyId[ptyId] : undefined, Date.now()),
      )
    read()
    const timer = window.setInterval(read, POLL_MS)
    return () => window.clearInterval(timer)
  }, [ptyId])

  return (
    <span className={styles.state} data-state={state}>
      <span className={styles.dot} aria-hidden="true" />
      {t(`team.state.${state}`)}
    </span>
  )
}
