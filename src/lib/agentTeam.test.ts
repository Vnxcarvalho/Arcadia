import { describe, expect, it } from 'vitest'

import { leadMessage, parseTeamRoles, teamHelperPrompt, teamPrompt } from './agentTeam'
import { buildAgentLaunch } from './sessionLaunch'

describe('parseTeamRoles', () => {
  it('splits on commas and lines, trims, and drops blanks and repeats', () => {
    expect(parseTeamRoles(' front, Back ,\n qa;; FRONT ')).toEqual(['front', 'Back', 'qa'])
  })

  it('drops quotes, which would break the command line the prompt travels on', () => {
    expect(parseTeamRoles(`"front", 'ba'ck`)).toEqual(['front', 'back'])
  })

  it('keeps at most six roles', () => {
    expect(parseTeamRoles('a,b,c,d,e,f,g,h')).toHaveLength(6)
  })

  it('returns nothing for an empty list', () => {
    expect(parseTeamRoles(' , ')).toEqual([])
  })
})

describe('teamPrompt', () => {
  it('tells a member its own area and where the team coordinates', () => {
    const prompt = teamPrompt('front', ['front', 'back'])
    expect(prompt).toContain('the front member')
    expect(prompt).toContain('TEAM.md')
    expect(prompt).not.toMatch(/["'\n]/)
  })

  it('tells the lead who is on the team', () => {
    const prompt = teamPrompt(null, ['front', 'back'])
    expect(prompt).toContain('lead')
    expect(prompt).toContain('front, back')
    expect(prompt).not.toMatch(/["'\n]/)
  })
})

describe('opening members on request', () => {
  it('teaches the lead the command and gives a new member its task', () => {
    expect(teamPrompt(null, ['front'])).toContain('alethe-team open ROLE TASK')
    const prompt = teamPrompt('back', ['front', 'back'], `check "the" form's rules`)
    expect(prompt).toContain('Your task: check the forms rules')
    expect(prompt).not.toMatch(/["'\n]/)
  })

  it('tells a plain terminal how to open a team, without quotes', () => {
    const prompt = teamHelperPrompt()
    expect(prompt).toContain('alethe-team open ROLE TASK')
    expect(prompt).toContain('alethe-team send ROLE TASK')
    expect(prompt).not.toMatch(/["'\n]/)
  })
})

describe('an Antigravity opening prompt', () => {
  it('adds the opening flags only to a conversation that starts fresh', () => {
    expect(
      buildAgentLaunch('antigravity', [], undefined, undefined, [], undefined, [], ['-i', 'x'])
        .args,
    ).toEqual(['-i', 'x'])
    expect(
      buildAgentLaunch('antigravity', [], 'conv', undefined, [], undefined, [], ['-i', 'x']).args,
    ).toEqual(['--conversation', 'conv'])
    expect(
      buildAgentLaunch(
        'antigravity',
        ['-i', 'mine'],
        undefined,
        undefined,
        [],
        undefined,
        [],
        ['-i', 'x'],
      ).args,
    ).toEqual(['-i', 'mine'])
  })

  it('is sent on the first launch', () => {
    expect(buildAgentLaunch('antigravity', ['-i', 'hello']).args).toEqual(['-i', 'hello'])
  })

  it('is not sent again when a conversation is resumed', () => {
    expect(buildAgentLaunch('antigravity', ['-i', 'hello', '--x'], 'conv-1').args).toEqual([
      '--conversation',
      'conv-1',
      '--x',
    ])
  })
})

describe('sending tasks to members that are already open', () => {
  it('teaches the lead to send every task, not only the first', () => {
    const prompt = teamPrompt(null, ['front', 'back'])
    expect(prompt).toContain('alethe-team send ROLE TASK')
    expect(prompt).toContain('for as many tasks as the user gives you')
    expect(prompt).toContain('never started')
    expect(prompt).not.toMatch(/["'\n]/)
  })

  it('tells a member that a message from the lead is a new task', () => {
    const prompt = teamPrompt('front', ['front', 'back'])
    expect(prompt).toContain('A message that starts with [lead] is a new task')
    expect(prompt).toContain('Keep taking new tasks after your first one')
    expect(prompt).not.toMatch(/["'\n]/)
  })

  it('marks what the lead sends and keeps it on one plain line', () => {
    expect(leadMessage(`add the "login" route\nand test it`)).toBe(
      '[lead] add the login route and test it',
    )
    expect(leadMessage('x'.repeat(5000))).toHaveLength(4000)
  })
})
