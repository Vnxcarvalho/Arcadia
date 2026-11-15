import type { Terminal } from './types'

/** Colours of the members of a team, taken from the theme so every theme tints them. */
export const TEAM_COLORS = [
  'var(--agent-antigravity)',
  'var(--status-working)',
  'var(--status-waiting)',
  'var(--agent-codex)',
  'var(--agent-kiro)',
  'var(--agent-claude)',
  'var(--agent-cursor)',
] as const

export type TeamMember = {
  terminalId: string
  /** The member's name: `lead`, `front`, `back`... */
  name: string
  /** 1 for the lead, then 2, 3... in the order the members were opened. */
  index: number
  color: string
  isLead: boolean
}

export type Team = {
  leadId: string
  members: TeamMember[]
}

function runsAntigravity(terminal: Terminal): boolean {
  return terminal.tabs.some((tab) => tab.type === 'antigravity')
}

/**
 * The Antigravity teams of a project: a lead and the terminals that name it as their leader
 * (`teamLeaderId`), the lead first, then the members in the order they were opened.
 */
export function antigravityTeams(terminals: readonly Terminal[]): Team[] {
  const teams: Team[] = []
  for (const lead of terminals) {
    if (lead.teamLeaderId || !runsAntigravity(lead)) continue
    const mates = terminals.filter((terminal) => terminal.teamLeaderId === lead.id)
    if (mates.length === 0) continue
    const members = [lead, ...mates].map((terminal, position) => ({
      terminalId: terminal.id,
      name: terminal.name,
      index: position + 1,
      color: TEAM_COLORS[position % TEAM_COLORS.length],
      isLead: position === 0,
    }))
    teams.push({ leadId: lead.id, members })
  }
  return teams
}

/** The member a terminal is in an Antigravity team, or null when it is not part of one. */
export function teamMemberOf(
  terminals: readonly Terminal[],
  terminalId: string,
): TeamMember | null {
  for (const team of antigravityTeams(terminals)) {
    const member = team.members.find((candidate) => candidate.terminalId === terminalId)
    if (member) return member
  }
  return null
}

/** What a member is doing, read from how long ago its terminal last printed something. */
export type MemberState = 'working' | 'ready' | 'ended'

const WORKING_WINDOW_MS = 2500

export function memberState(
  runtime: { alive: boolean; lastIoAt: number } | undefined,
  now: number,
): MemberState {
  if (!runtime || !runtime.alive) return 'ended'
  return now - runtime.lastIoAt < WORKING_WINDOW_MS ? 'working' : 'ready'
}
