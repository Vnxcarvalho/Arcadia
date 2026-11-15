import { beforeEach, describe, expect, it, vi } from 'vitest'

import { useProjectsStore } from '../stores/projectsStore'
import { findTeamMember, sendToTeamMember } from './antigravityTeam'
import type { Project, Terminal } from './types'

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }))
vi.mock('./tauri', () => ({ writePty: vi.fn() }))

function terminal(id: string, name: string, ptyId: string | null, teamLeaderId?: string): Terminal {
  return {
    id,
    name,
    teamLeaderId,
    tabs: [{ id: `${id}-tab`, ptyId }],
  } as unknown as Terminal
}

function project(terminals: Terminal[]): Project {
  return { id: 'project-1', terminals } as unknown as Project
}

describe('finding the member a lead means', () => {
  const lead = terminal('lead', 'lead', 'pty-lead')
  const front = terminal('front', 'front', 'pty-front', 'lead')
  const back = terminal('back', 'Back', 'pty-back', 'lead')
  const other = terminal('other', 'front', 'pty-other', 'someone-else')
  const projects = [project([lead, front, back, other])]

  it('matches the role by name, ignoring case, inside the lead own team', () => {
    expect(findTeamMember(projects, 'pty-lead', 'FRONT')?.id).toBe('front')
    expect(findTeamMember(projects, 'pty-lead', 'back')?.id).toBe('back')
  })

  it('treats a member that asks as its own lead asking', () => {
    expect(findTeamMember(projects, 'pty-front', 'back')?.id).toBe('back')
  })

  it('finds nothing for an unknown role or caller', () => {
    expect(findTeamMember(projects, 'pty-lead', 'qa')).toBeNull()
    expect(findTeamMember(projects, 'pty-unknown', 'front')).toBeNull()
  })
})

describe('sending a task to a member', () => {
  beforeEach(() => {
    useProjectsStore.setState({
      projects: [
        project([
          terminal('lead', 'lead', 'pty-lead'),
          terminal('front', 'front', 'pty-front', 'lead'),
          terminal('idle', 'back', null, 'lead'),
        ]),
      ],
    } as never)
  })

  it('types the task and then presses Enter, for the second task as well as the first', async () => {
    const write = vi.fn().mockResolvedValue(undefined)
    const wait = vi.fn().mockResolvedValue(undefined)
    const payload = { leaderPtyId: 'pty-lead', role: 'front', message: 'fix the menu' }

    expect(await sendToTeamMember(payload, write, wait)).toBe(true)
    expect(await sendToTeamMember({ ...payload, message: 'now the footer' }, write, wait)).toBe(
      true,
    )

    expect(write.mock.calls).toEqual([
      ['pty-front', '[lead] fix the menu'],
      ['pty-front', '\r'],
      ['pty-front', '[lead] now the footer'],
      ['pty-front', '\r'],
    ])
    expect(wait).toHaveBeenCalledTimes(2)
  })

  it('reports false, and writes nothing, when the member is not running', async () => {
    const write = vi.fn()
    const wait = vi.fn()
    expect(
      await sendToTeamMember({ leaderPtyId: 'pty-lead', role: 'back', message: 'x' }, write, wait),
    ).toBe(false)
    expect(
      await sendToTeamMember({ leaderPtyId: 'pty-lead', role: 'qa', message: 'x' }, write, wait),
    ).toBe(false)
    expect(write).not.toHaveBeenCalled()
  })
})
