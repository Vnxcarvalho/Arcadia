import type { TFunction } from './i18n'

/** Git says one of these when the repository is private or missing and no sign-in was offered. */
const NO_ACCESS =
  /repository .*not found|authentication failed|could not read username|permission denied|access denied|403|401/i

/**
 * The sentence a failed clone shows, in the language of the app. The backend answers with a short
 * code (`clone_target_not_empty:<path>`, `clone_failed:<git's message>`, `clone_git_missing:<why>`).
 */
export function describeCloneError(error: unknown, t: TFunction): string {
  const text = String(error)
  const notEmpty = text.match(/clone_target_not_empty:(.+)$/s)
  if (notEmpty) return t('crud.cloneFolderTaken', { path: notEmpty[1].trim() })
  if (text.includes('clone_git_missing:')) return t('crud.cloneGitMissing')
  const failed = text.match(/clone_failed:(.*)$/s)
  const reason = (failed ? failed[1] : text).trim()
  if (NO_ACCESS.test(reason)) return t('crud.cloneNoAccess')
  // Git's own sentence, without its `fatal:` prefix and on one line.
  const oneLine = reason
    .replace(/^(fatal|error):\s*/i, '')
    .split('\n')[0]
    .trim()
  return t('crud.cloneFailedReason', { reason: oneLine || text })
}
