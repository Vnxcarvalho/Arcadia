import { useEffect, useMemo, useState } from 'react'

import { useT } from '../../lib/i18n'
import { antigravityTeams } from '../../lib/teamView'
import type { Project } from '../../lib/types'
import { useUiStore } from '../../stores/uiStore'
import styles from './TeamStatusBar.module.css'

const CLOCK_MS = 30_000

function clockText(date: Date): string {
  return `${String(date.getHours()).padStart(2, '0')}:${String(date.getMinutes()).padStart(2, '0')}`
}

/**
 * The status line under an Antigravity team, as tmux draws one: the session name, the members as
 * numbered windows with the focused one starred, and the time. A member can be clicked to focus it.
 */
export function TeamStatusBar({ project }: { project: Project }) {
  const t = useT()
  const teams = useMemo(() => antigravityTeams(project.terminals), [project.terminals])
  const focusedId = useUiStore((s) => s.activeTerminal?.terminalId ?? null)
  const requestPaneFocus = useUiStore((s) => s.requestPaneFocus)
  const [clock, setClock] = useState(() => new Date())

  useEffect(() => {
    const timer = window.setInterval(() => setClock(new Date()), CLOCK_MS)
    return () => window.clearInterval(timer)
  }, [])

  const team = teams[0]
  if (!team) return null

  return (
    <div className={styles.bar} role="toolbar" aria-label={t('team.bar')}>
      <span className={styles.session} title={project.name}>
        {project.name}
      </span>
      <span className={styles.count}>{t('team.agentCount', { count: team.members.length })}</span>
      <nav className={styles.windows}>
        {team.members.map((member) => {
          const focused = member.terminalId === focusedId
          return (
            <button
              key={member.terminalId}
              type="button"
              className={`${styles.window} ${focused ? styles.windowFocused : ''}`}
              style={{ ['--member-color' as string]: member.color }}
              onClick={() => requestPaneFocus(member.terminalId)}
              title={member.name}
            >
              <span className={styles.dot} aria-hidden="true" />
              {member.index}:{member.name}
              {focused ? '*' : ''}
            </button>
          )
        })}
      </nav>
      <span className={styles.clock}>{clockText(clock)}</span>
    </div>
  )
}
