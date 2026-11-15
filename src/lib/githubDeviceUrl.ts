/**
 * The GitHub page that authorizes a device-flow sign-in, with the code already filled in. GitHub
 * skips its "enter the code" step when `user_code` is in the address, so the person only confirms
 * the account and authorizes Arcadia on the website.
 */
export function githubAuthorizeUrl(start: { verification_uri: string; user_code: string }): string {
  return `${start.verification_uri}?user_code=${encodeURIComponent(start.user_code)}`
}
