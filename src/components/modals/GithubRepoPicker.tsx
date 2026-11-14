import { Github, Loader2, Lock } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'

import { useT } from '../../lib/i18n'
import {
  filterGithubRepos,
  githubAccountDeviceFinish,
  githubAccountDeviceStart,
  githubAccountLogout,
  githubAccountRepos,
  type GithubAccountStatus,
  githubAccountStatus,
  type GithubDeviceStart,
  type GithubRepo,
  openInBrowser,
} from '../../lib/tauri'
import controls from './controls.module.css'
import styles from './GithubRepoPicker.module.css'

type MessageKey = Parameters<ReturnType<typeof useT>>[0]

function signInErrorKey(error: unknown): MessageKey {
  const text = String(error)
  if (text.includes('access_denied')) return 'githubAccount.denied'
  if (text.includes('code_expired')) return 'githubAccount.expired'
  if (text.includes('client_id_missing')) return 'githubAccount.notConfigured'
  return 'githubAccount.failed'
}

/** Sign in with GitHub and pick one of the account's repositories to clone. */
export function GithubRepoPicker({ onPick }: { onPick: (repo: GithubRepo) => void }) {
  const t = useT()
  // `useT()` returns a new function on every render; keeping it out of the effect dependencies stops
  // the repository list from being requested again after each render.
  const tRef = useRef(t)
  tRef.current = t
  const [status, setStatus] = useState<GithubAccountStatus | null>(null)
  const [device, setDevice] = useState<GithubDeviceStart | null>(null)
  const [repos, setRepos] = useState<GithubRepo[] | null>(null)
  const [query, setQuery] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const loadRepos = useCallback(async () => {
    setError(null)
    setRepos(null)
    try {
      setRepos(await githubAccountRepos())
    } catch (err) {
      if (String(err).includes('session_expired')) {
        setStatus(await githubAccountStatus())
        setError(tRef.current('githubAccount.sessionExpired'))
      } else {
        setError(tRef.current('githubAccount.loadFailed', { error: String(err) }))
      }
      setRepos([])
    }
  }, [])

  useEffect(() => {
    let cancelled = false
    void githubAccountStatus().then((next) => {
      if (cancelled) return
      setStatus(next)
      if (next.connected) void loadRepos()
    })
    return () => {
      cancelled = true
    }
  }, [loadRepos])

  const signIn = async () => {
    setBusy(true)
    setError(null)
    try {
      const start = await githubAccountDeviceStart()
      setDevice(start)
      void openInBrowser(start.verification_uri)
      const next = await githubAccountDeviceFinish(start)
      setStatus(next)
      if (next.connected) void loadRepos()
    } catch (err) {
      setError(t(signInErrorKey(err)))
    } finally {
      setDevice(null)
      setBusy(false)
    }
  }

  const signOut = async () => {
    setStatus(await githubAccountLogout())
    setRepos(null)
    setQuery('')
  }

  const visible = useMemo(() => filterGithubRepos(repos ?? [], query), [repos, query])

  if (!status) return null

  if (!status.configured) {
    return (
      <div className={styles.panel}>
        <p className={styles.note}>{t('githubAccount.notConfigured')}</p>
      </div>
    )
  }

  if (!status.connected) {
    return (
      <div className={styles.panel}>
        {device ? (
          <div className={styles.device}>
            <p className={styles.note}>{t('githubAccount.codeHint')}</p>
            <span className={styles.code}>{device.user_code}</span>
            <button
              type="button"
              className={styles.link}
              onClick={() => void openInBrowser(device.verification_uri)}
            >
              {t('githubAccount.openGithub')}
            </button>
          </div>
        ) : (
          <button
            type="button"
            className={controls.btn}
            disabled={busy}
            onClick={() => void signIn()}
          >
            <Github size={14} aria-hidden="true" />
            {busy ? t('githubAccount.waiting') : t('githubAccount.signIn')}
          </button>
        )}
        {error ? <p className={styles.error}>{error}</p> : null}
      </div>
    )
  }

  return (
    <div className={styles.panel}>
      <div className={styles.account}>
        {status.avatar_url ? (
          <img className={styles.avatar} src={status.avatar_url} alt="" />
        ) : null}
        <span className={styles.accountName}>
          {t('githubAccount.signedInAs', { login: status.login ?? '' })}
        </span>
        <button type="button" className={styles.link} onClick={() => void signOut()}>
          {t('githubAccount.signOut')}
        </button>
      </div>
      <input
        className={controls.input}
        value={query}
        onChange={(event) => setQuery(event.target.value)}
        placeholder={t('githubAccount.search')}
        aria-label={t('githubAccount.search')}
      />
      {error ? <p className={styles.error}>{error}</p> : null}
      {repos === null ? (
        <p className={styles.note}>
          <Loader2 size={12} className={styles.spin} aria-hidden="true" />{' '}
          {t('githubAccount.loading')}
        </p>
      ) : visible.length === 0 ? (
        <p className={styles.note}>{t('githubAccount.empty')}</p>
      ) : (
        <ul className={styles.list}>
          {visible.map((repo) => (
            <li key={repo.id}>
              <button type="button" className={styles.repo} onClick={() => onPick(repo)}>
                <span className={styles.repoName}>
                  {repo.full_name}
                  {repo.private ? (
                    <Lock
                      size={11}
                      aria-label={t('githubAccount.private')}
                      className={styles.lock}
                    />
                  ) : null}
                </span>
                {repo.description ? (
                  <span className={styles.repoDescription}>{repo.description}</span>
                ) : null}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
