/**
 * Accepts Antigravity's permission prompt for Arcadia's own team helper.
 *
 * A lead opens a member, or sends it a task, by running `alethe-team open|send ...` in its shell.
 * Antigravity asks the person before it runs any command, and on Windows its saved rules only
 * match an identical command line, so every new role or task asked again. Only that one command is
 * accepted here, only when the whole command is shown on the screen and holds no shell operators;
 * any other command still waits for the person.
 */

/** The role is one word; the task is plain text without shell operators, quotes or line breaks. */
const TEAM_COMMAND =
  /^alethe-team (?:open|send) [A-Za-z0-9_-]{1,24}(?: [^;&|<>`$(){}\\^"'\r\n]{0,4000})?$/

const PROMPT_TITLE = /Run this command\?/
const REQUEST_LINE = /Requesting permission for:/
/** The menu entry that is highlighted by default, which Enter confirms. */
const DEFAULT_YES = /^\s*[>❯]\s*1\.\s*Yes, run command\s*$/

/**
 * The team command a permission prompt on `screen` asks about, or null when the screen shows no
 * such prompt, shows a different command, or the highlighted option is not the plain "Yes".
 */
export function findTeamCommandPrompt(screen: string): string | null {
  const lines = screen.split('\n').map((line) => line.replace(/\s+$/, ''))
  let title = -1
  for (let index = lines.length - 1; index >= 0; index -= 1) {
    if (PROMPT_TITLE.test(lines[index])) {
      title = index
      break
    }
  }
  if (title < 0) return null

  let request = -1
  for (let index = title - 1; index >= 0; index -= 1) {
    if (REQUEST_LINE.test(lines[index])) {
      request = index
      break
    }
  }
  if (request < 0) return null

  const command = lines
    .slice(request + 1, title)
    .map((line) => line.trim())
    .filter(Boolean)
    .join(' ')
  if (!TEAM_COMMAND.test(command)) return null

  const menu = lines.slice(title + 1, title + 8)
  return menu.some((line) => DEFAULT_YES.test(line)) ? command : null
}

type Deps = {
  /** The text of the rows now on the screen, with every escape code already applied. */
  readScreen: () => string
  /** Presses Enter in the terminal. */
  confirm: () => Promise<void>
  now?: () => number
}

const SETTLE_MS = 300
/** The same prompt is not answered twice in a row inside this window. */
const REPEAT_GUARD_MS = 4_000

/** Watches one Antigravity terminal and confirms the prompt for the team helper when it shows up. */
export class AntigravityTeamApproval {
  private timer: number | null = null
  private lastCommand = ''
  private lastAnsweredAt = 0
  private disposed = false

  constructor(private readonly deps: Deps) {}

  /** Call after the terminal received output. Waits for the screen to settle before reading it. */
  handleOutput(): void {
    if (this.disposed) return
    if (this.timer !== null) window.clearTimeout(this.timer)
    this.timer = window.setTimeout(() => {
      this.timer = null
      this.check()
    }, SETTLE_MS)
  }

  /** Reads the screen once. Exposed for tests. */
  check(): boolean {
    if (this.disposed) return false
    const command = findTeamCommandPrompt(this.deps.readScreen())
    if (!command) return false
    const now = (this.deps.now ?? Date.now)()
    if (command === this.lastCommand && now - this.lastAnsweredAt < REPEAT_GUARD_MS) return false
    this.lastCommand = command
    this.lastAnsweredAt = now
    void this.deps.confirm().catch(() => undefined)
    return true
  }

  dispose(): void {
    this.disposed = true
    if (this.timer !== null) window.clearTimeout(this.timer)
    this.timer = null
  }
}
