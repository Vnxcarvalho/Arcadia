import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const tauri = vi.hoisted(() => ({
  filterGithubRepos: (repos: unknown[]) => repos,
  githubAccountStatus: vi.fn(),
  githubAccountDeviceStart: vi.fn(),
  githubAccountDeviceFinish: vi.fn(),
  githubAccountLogout: vi.fn(),
  githubAccountRepos: vi.fn(),
  openInBrowser: vi.fn(),
}))

vi.mock('../../lib/tauri', () => tauri)

vi.mock('../../stores/projectsStore', () => ({
  useProjectsStore: (selector: (state: unknown) => unknown) =>
    selector({ preferences: { language: 'en' } }),
}))

import { GithubRepoPicker } from './GithubRepoPicker'

const repo = {
  id: 1,
  full_name: 'me/app',
  name: 'app',
  owner: 'me',
  description: 'An app',
  private: true,
  fork: false,
  archived: false,
  updated_at: null,
  clone_url: 'https://github.com/me/app.git',
}

describe('GithubRepoPicker', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    tauri.githubAccountStatus.mockResolvedValue({
      configured: true,
      connected: true,
      login: 'me',
      name: null,
      avatar_url: null,
    })
    tauri.githubAccountRepos.mockResolvedValue([repo])
  })

  afterEach(cleanup)

  it('requests the repositories once, not again after every render', async () => {
    render(<GithubRepoPicker onPick={vi.fn()} />)

    await screen.findByText('me/app')
    // Let any stray re-render settle before counting the requests.
    await new Promise((resolve) => setTimeout(resolve, 50))

    expect(tauri.githubAccountStatus).toHaveBeenCalledTimes(1)
    expect(tauri.githubAccountRepos).toHaveBeenCalledTimes(1)
  })

  it('shows the message when the repositories cannot be loaded', async () => {
    tauri.githubAccountRepos.mockRejectedValue('github returned 403: API rate limit exceeded')

    render(<GithubRepoPicker onPick={vi.fn()} />)

    await waitFor(() => expect(screen.getByText(/API rate limit exceeded/)).toBeTruthy())
    expect(tauri.githubAccountRepos).toHaveBeenCalledTimes(1)
  })
  it('shows the sign-in page address, and says so when the browser cannot be opened', async () => {
    tauri.githubAccountStatus.mockResolvedValue({
      configured: true,
      connected: false,
      login: null,
      name: null,
      avatar_url: null,
    })
    tauri.githubAccountDeviceStart.mockResolvedValue({
      device_code: 'device',
      user_code: 'ABCD-1234',
      verification_uri: 'https://github.com/login/device',
      interval: 5,
      expires_in: 900,
    })
    tauri.githubAccountDeviceFinish.mockReturnValue(new Promise(() => {}))
    tauri.openInBrowser.mockRejectedValue('no default browser')

    render(<GithubRepoPicker onPick={vi.fn()} />)
    fireEvent.click(await screen.findByRole('button', { name: /Sign in with GitHub/ }))

    expect(await screen.findByText('ABCD-1234')).toBeTruthy()
    expect(screen.getByText('https://github.com/login/device')).toBeTruthy()
    await waitFor(() => expect(screen.getByText(/Could not open the browser/)).toBeTruthy())
  })
})
