import { describe, expect, it } from 'vitest'

import { githubAuthorizeUrl } from './githubDeviceUrl'

describe('githubAuthorizeUrl', () => {
  it('puts the code in the address so GitHub does not ask for it', () => {
    expect(
      githubAuthorizeUrl({
        verification_uri: 'https://github.com/login/device',
        user_code: 'ABCD-1234',
      }),
    ).toBe('https://github.com/login/device?user_code=ABCD-1234')
  })

  it('escapes anything that is not a plain code character', () => {
    expect(
      githubAuthorizeUrl({
        verification_uri: 'https://github.com/login/device',
        user_code: 'A B&C',
      }),
    ).toBe('https://github.com/login/device?user_code=A%20B%26C')
  })
})
