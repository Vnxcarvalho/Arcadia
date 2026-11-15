/** How long a request for the keyboard stays fresh, which covers a window closing around it. */
const FRESH_MS = 1500

/**
 * Whether a terminal asked for the keyboard a moment ago. A window that opened that terminal must
 * not give the keyboard back to the button that opened the window.
 */
export function terminalFocusWasJustRequested(
  request: { ts: number } | null,
  now: number = Date.now(),
): boolean {
  return request !== null && now - request.ts >= 0 && now - request.ts < FRESH_MS
}
