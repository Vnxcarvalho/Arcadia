import { describe, expect, it } from 'vitest'

import { antigravityTeams, memberState, TEAM_COLORS, teamMemberOf } from './teamView'
import type { Terminal } from './types'

function terminal(id: string, name: string, type: string, teamLeaderId?: string): Terminal {
  return { id, name, teamLeaderId, tabs: [{ id: `${id}-tab`, type, ptyId: `pty-${id}` }] } as never
}

describe('antigravityTeams', () => {
  const terminals = [
    terminal('lead', 'lead', 'antigravity'),
    terminal('front', 'front', 'antigravity', 'lead'),
    terminal('back', 'back', 'antigravity', 'lead'),
    terminal('alone', 'shell', 'shell'),
  ]

  it('lists the lead first, then the members in the order they were opened', () => {
    const [team] = antigravityTeams(terminals)
    expect(team.leadId).toBe('lead')
    expect(team.members.map((member) => `${member.index}:${member.name}`)).toEqual([
      '1:lead',
      '2:front',
      '3:back',
    ])
    expect(team.members.map((member) => member.isLead)).toEqual([true, false, false])
  })

  it('gives each member its own colour from the theme', () => {
    const [team] = antigravityTeams(terminals)
    expect(team.members.map((member) => member.color)).toEqual(TEAM_COLORS.slice(0, 3))
    expect(new Set(team.members.map((member) => member.color)).size).toBe(3)
  })

  it('finds no team in a terminal that has no members, or in one that is not Antigravity', () => {
    expect(antigravityTeams([terminal('a', 'a', 'antigravity')])).toEqual([])
    expect(
      antigravityTeams([terminal('l', 'l', 'claude'), terminal('m', 'm', 'claude', 'l')]),
    ).toEqual([])
  })
})

describe('teamMemberOf', () => {
  const terminals = [
    terminal('lead', 'lead', 'antigravity'),
    terminal('front', 'front', 'antigravity', 'lead'),
    terminal('alone', 'shell', 'shell'),
  ]

  it('says which member a terminal is, and null for one outside a team', () => {
    expect(teamMemberOf(terminals, 'front')?.index).toBe(2)
    expect(teamMemberOf(terminals, 'lead')?.isLead).toBe(true)
    expect(teamMemberOf(terminals, 'alone')).toBeNull()
    expect(teamMemberOf(terminals, 'missing')).toBeNull()
  })
})

describe('memberState', () => {
  it('is working while the terminal printed something a moment ago, then ready', () => {
    expect(memberState({ alive: true, lastIoAt: 10_000 }, 11_000)).toBe('working')
    expect(memberState({ alive: true, lastIoAt: 10_000 }, 20_000)).toBe('ready')
  })

  it('is ended when the terminal is gone or not running', () => {
    expect(memberState(undefined, 1_000)).toBe('ended')
    expect(memberState({ alive: false, lastIoAt: 999 }, 1_000)).toBe('ended')
  })
})
