import { describe, expect, it } from 'vitest'

import { describeCloneError } from './cloneError'
import type { TFunction } from './i18n'

// Echoes the key and the values, so the test checks which sentence is picked.
const t = ((key: string, params?: Record<string, unknown>) =>
  params ? `${key}:${Object.values(params).join(',')}` : key) as unknown as TFunction

describe('describeCloneError', () => {
  it('says the folder is taken, with its path', () => {
    expect(describeCloneError('clone_target_not_empty:C:\\Users\\me\\Alethe\\repo', t)).toBe(
      'crud.cloneFolderTaken:C:\\Users\\me\\Alethe\\repo',
    )
  })

  it('points to the sign-in when GitHub refuses or cannot find the repository', () => {
    for (const reason of [
      "fatal: repository 'https://github.com/a/b.git/' not found",
      "fatal: Authentication failed for 'https://github.com/a/b.git/'",
      'fatal: could not read Username for https://github.com: terminal prompts disabled',
    ]) {
      expect(describeCloneError(`clone_failed:${reason}`, t)).toBe('crud.cloneNoAccess')
    }
  })

  it('shows git own sentence, on one line and without its prefix, for anything else', () => {
    expect(
      describeCloneError('clone_failed:fatal: unable to access the network\nhint: check it', t),
    ).toBe('crud.cloneFailedReason:unable to access the network')
  })

  it('tells when git is missing, and still handles a message that is not a code', () => {
    expect(describeCloneError('clone_git_missing:program not found', t)).toBe(
      'crud.cloneGitMissing',
    )
    expect(describeCloneError(new Error('boom'), t)).toBe('crud.cloneFailedReason:boom')
  })
})
