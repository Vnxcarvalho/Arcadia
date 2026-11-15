import { listen, type UnlistenFn } from '@tauri-apps/api/event'

import { useProjectsStore } from '../stores/projectsStore'
import { leadMessage, parseTeamRoles, teamPrompt } from './agentTeam'
import { writePty } from './tauri'
import { findPtyOwner, mainVerticalLayout } from './tmuxBridge'
import type { Project, Terminal } from './types'

/** `agy -i` starts the session with this prompt and stays interactive. */
const INITIAL_PROMPT_FLAG = '-i'
/** Sent by `tmux_bridge.rs` when a lead runs `alethe-team open <role> [task]`. */
const OPEN_MEMBER_EVENT = 'agent-team-open'
/** Sent by `tmux_bridge.rs` when a lead runs `alethe-team send <role> <message>`. */
const SEND_MEMBER_EVENT = 'agent-team-send'
/** Time between typing a task and pressing Enter, so the agent takes it as one submission. */
const SUBMIT_DELAY_MS = 150

type OpenMemberPayload = { leaderPtyId: string; role: string; task?: string | null }
type SendMemberPayload = { leaderPtyId: string; role: string; message: string }

/** Opens one Antigravity terminal named `name`, with `role` (null for the lead) as its first prompt. */
async function openMember(args: {
  projectId: string
  cwd: string
  name: string
  role: string | null
  roles: readonly string[]
  task?: string
  teamLeaderId?: string
  extraArgs?: string[]
}): Promise<Terminal> {
  const { projectId, cwd, name, role, roles, task, teamLeaderId, extraArgs = [] } = args
  const store = useProjectsStore.getState()
  const terminal = await store.createAgentTerminal(projectId, {
    name,
    cwd,
    teamLeaderId,
    firstTab: {
      type: 'antigravity',
      cwd,
      extraArgs: [...extraArgs, INITIAL_PROMPT_FLAG, teamPrompt(role, roles, task, cwd)],
    },
  })
  // A terminal is labelled with its agent until it is renamed.
  store.renameTerminal(projectId, terminal.id, name)
  return terminal
}

/**
 * Opens a lead and one terminal per role, all running Antigravity, with the lead on the left and
 * the team stacked on the right. Closing the lead closes the team (`teamLeaderId`).
 */
export async function createAntigravityTeam(args: {
  projectId: string
  cwd: string
  roles: readonly string[]
  extraArgs?: string[]
}): Promise<string> {
  const { projectId, cwd, roles, extraArgs } = args
  const lead = await openMember({ projectId, cwd, name: 'lead', role: null, roles, extraArgs })
  const mates: Terminal[] = []
  for (const role of roles) {
    mates.push(
      await openMember({
        projectId,
        cwd,
        name: role,
        role,
        roles,
        teamLeaderId: lead.id,
        extraArgs,
      }),
    )
  }
  useProjectsStore.getState().setProjectGridLayout(
    projectId,
    mainVerticalLayout(
      lead.id,
      mates.map((mate) => mate.id),
    ),
  )
  return lead.id
}

/**
 * A lead asked, from its terminal, for one more member: opens it next to the lead and lays the
 * team out again. A role the team already has is left as it is.
 */
export async function openTeamMember(payload: OpenMemberPayload): Promise<void> {
  const [role] = parseTeamRoles(payload.role)
  if (!role) return
  const store = useProjectsStore.getState()
  const owner = findPtyOwner(store.projects, payload.leaderPtyId)
  if (!owner) return
  const project = store.projects.find((candidate) => candidate.id === owner.projectId)
  if (!project) return
  // A member that asks is treated as the team's own lead asking.
  const lead =
    project.terminals.find((terminal) => terminal.id === owner.terminal.teamLeaderId) ??
    owner.terminal
  const members = project.terminals.filter((terminal) => terminal.teamLeaderId === lead.id)
  if (members.some((member) => member.name.toLowerCase() === role.toLowerCase())) return
  const roles = [...members.map((member) => member.name), role]
  const cwd = lead.cwd || owner.terminal.cwd
  const created = await openMember({
    projectId: project.id,
    cwd,
    name: role,
    role,
    roles,
    task: payload.task ?? undefined,
    teamLeaderId: lead.id,
  })
  useProjectsStore
    .getState()
    .setProjectGridLayout(
      project.id,
      mainVerticalLayout(lead.id, [...members.map((member) => member.id), created.id]),
    )
}

/** The terminal of the member called `role` in the team that the terminal running `leaderPtyId` leads. */
export function findTeamMember(
  projects: Project[],
  leaderPtyId: string,
  role: string,
): Terminal | null {
  const owner = findPtyOwner(projects, leaderPtyId)
  if (!owner) return null
  const project = projects.find((candidate) => candidate.id === owner.projectId)
  if (!project) return null
  const lead =
    project.terminals.find((terminal) => terminal.id === owner.terminal.teamLeaderId) ??
    owner.terminal
  const wanted = role.trim().toLowerCase()
  return (
    project.terminals.find(
      (terminal) =>
        terminal.teamLeaderId === lead.id && terminal.name.trim().toLowerCase() === wanted,
    ) ?? null
  )
}

/**
 * A lead sent a task to a member that is already open: types it into the member's terminal and
 * presses Enter, so the member starts on it right away. False when the member is not running.
 */
export async function sendToTeamMember(
  payload: SendMemberPayload,
  write: (ptyId: string, data: string) => Promise<void> = writePty,
  wait: (ms: number) => Promise<void> = (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
): Promise<boolean> {
  const member = findTeamMember(
    useProjectsStore.getState().projects,
    payload.leaderPtyId,
    payload.role,
  )
  const ptyId = member?.tabs.find((tab) => tab.ptyId)?.ptyId
  if (!ptyId) return false
  await write(ptyId, leadMessage(payload.message))
  await wait(SUBMIT_DELAY_MS)
  await write(ptyId, '\r')
  return true
}

/** Listens for members a lead asks Arcadia to open, and for tasks it sends to members already open. */
export function listenAgentTeamOpen(): () => void {
  const stopOpen: Promise<UnlistenFn> = listen<OpenMemberPayload>(
    OPEN_MEMBER_EVENT,
    ({ payload }) => {
      void openTeamMember(payload)
    },
  )
  const stopSend: Promise<UnlistenFn> = listen<SendMemberPayload>(
    SEND_MEMBER_EVENT,
    ({ payload }) => {
      void sendToTeamMember(payload).catch((error) => {
        console.error('[Arcadia] could not send the task to the team member:', error)
      })
    },
  )
  return () => {
    void stopOpen.then((unlisten) => unlisten())
    void stopSend.then((unlisten) => unlisten())
  }
}
