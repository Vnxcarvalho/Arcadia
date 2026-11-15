import { describe, expect, it, vi } from 'vitest'

import { AntigravityTeamApproval, findTeamCommandPrompt } from './antigravityTeamApproval'

function prompt(command: string[], selected = '> 1. Yes, run command'): string {
  return [
    'Reason: the auto-run safety check did not reach a decision',
    'Requesting permission for:',
    ...command.map((line) => `   ${line}`),
    '',
    'Run this command?',
    selected,
    '  2. Yes, and always allow in this conversation for commands that start with x',
    '  3. Yes, and always allow for commands that start with x (Persist to settings.json)',
    '  4. No, cancel',
    '',
    '  up/down Navigate - tab Amend',
  ].join('\n')
}

describe('finding the prompt for the team helper', () => {
  it('reads an open command and a send command', () => {
    expect(
      findTeamCommandPrompt(prompt(['alethe-team open front Responsavel pelo novo design'])),
    ).toBe('alethe-team open front Responsavel pelo novo design')
    expect(findTeamCommandPrompt(prompt(['alethe-team send back add the login route']))).toBe(
      'alethe-team send back add the login route',
    )
  })

  it('joins a command the terminal wrapped over several lines', () => {
    expect(
      findTeamCommandPrompt(
        prompt(['alethe-team open front Responsavel pelo novo', 'design do frontend']),
      ),
    ).toBe('alethe-team open front Responsavel pelo novo design do frontend')
  })

  it('ignores any other command', () => {
    expect(findTeamCommandPrompt(prompt(['git push origin main']))).toBeNull()
    expect(findTeamCommandPrompt(prompt(['alethe-team close front']))).toBeNull()
    expect(findTeamCommandPrompt(prompt(['alethe-team-evil open front x']))).toBeNull()
  })

  it('refuses a command that chains something else or uses shell syntax', () => {
    for (const task of [
      'alethe-team open front x; del important.txt',
      'alethe-team open front x && curl evil',
      'alethe-team open front x | more',
      'alethe-team open front $(whoami)',
      'alethe-team open front `whoami`',
      'alethe-team open front x > out.txt',
      'alethe-team send back "quoted"',
      'alethe-team open bad.role x',
    ]) {
      expect(findTeamCommandPrompt(prompt([task]))).toBeNull()
    }
  })

  it('does nothing when the highlighted option is not the plain yes, or there is no prompt', () => {
    expect(
      findTeamCommandPrompt(prompt(['alethe-team open front x'], '> 4. No, cancel')),
    ).toBeNull()
    expect(findTeamCommandPrompt('alethe-team open front x\n$ ')).toBeNull()
    expect(findTeamCommandPrompt('')).toBeNull()
  })
})

describe('the watcher', () => {
  it('presses Enter once for a team prompt, not again right away', () => {
    const confirm = vi.fn().mockResolvedValue(undefined)
    let clock = 1_000
    const screen = prompt(['alethe-team open front x'])
    const watcher = new AntigravityTeamApproval({
      readScreen: () => screen,
      confirm,
      now: () => clock,
    })

    expect(watcher.check()).toBe(true)
    expect(watcher.check()).toBe(false)
    clock += 5_000
    expect(watcher.check()).toBe(true)
    expect(confirm).toHaveBeenCalledTimes(2)
  })

  it('never presses Enter for another command', () => {
    const confirm = vi.fn()
    const watcher = new AntigravityTeamApproval({
      readScreen: () => prompt(['rm -rf .']),
      confirm,
    })
    expect(watcher.check()).toBe(false)
    expect(confirm).not.toHaveBeenCalled()
  })

  it('stops after it is disposed', () => {
    const confirm = vi.fn()
    const watcher = new AntigravityTeamApproval({
      readScreen: () => prompt(['alethe-team open front x']),
      confirm,
    })
    watcher.dispose()
    expect(watcher.check()).toBe(false)
    expect(confirm).not.toHaveBeenCalled()
  })
})
