import type { BackgroundTaskSummary, On, ProcessRunResult, SessionCompactResult, SessionCronSummary, SessionMessage } from 'claude-code'
import { type Engine, expect, mock, test } from 'claude-code/testing'
import { armOf, assignment, CAPS, carried, carriedBlocks, commitsBlock, type Heard, type Item, itemsBlock, lands, mailBlock, saidBlock, stateBlock, type Workspace } from '../hooks/kit'

const HOME = '/home/u'
const LINK = `${HOME}/.local/state/claude-lanes/lane-a`
const STATE = '/w/lanes/lane-a.md'
const NOTES = '/w/notes.md'
const STATE_TEXT = `---\ncarry:\n  - ${NOTES}\n  - /w/gone.md\n---\n# lane-a: state\n\n## Goal\n- ship it\n`
const MAIL = `${HOME}/.local/state/agent-mail/sessions`
const TOOL = 'mcp__lane__rotate'
const CARRY_ON = 'This context started over at a rotation. Carry on from your state file and the blocks just before this message: they are all that came across.'
const CARRY_ON_B = 'This context started over at a rotation. Carry on from the blocks just before this message: they are all that came across.'
const AB = `${HOME}/.local/state/claude-lanes/ab.json`
const CHECKS = `${HOME}/.local/state/claude-lanes/checks`
const CONTEXTS = `${HOME}/.local/state/claude-lanes/contexts/lane-a.jsonl`
const CLAUDE_BIN = '/nix/store/x-claude-code-2.1.322/bin/.claude-wrapped'
const DAY = 86_400_000
/** With seed s3, lane-a's coin puts it in arm B first and lane-b's in arm A: SHA-256 of "s3\nlane-a" begins b9, of
 * "s3\nlane-b" d0 (by sha256sum). Every session switches a day in. */
const B_FIRST = JSON.stringify({ seed: 's3', switchAt: '1970-01-02T00:00:00Z' })

const commit = (change: string | undefined, id: string, at: string, subject: string, bookmarks: string[], pushed: boolean, empty = false) => ({
  ...(change !== undefined && { change }),
  commit: id,
  at,
  subject,
  bookmarks,
  pushed,
  empty,
})
const JJ: Workspace = {
  root: '/w',
  vcs: 'jj',
  head: commit('zzzzzzzz', 'aaaa1111', '2026-11-09T12:00:00Z', '', [], false, true),
  line: [commit('yyyyyyyy', 'bbbb2222', '2026-11-09T11:00:00Z', 'mods/lane: arm B', ['harness/v12-lane-mod'], true)],
  more: 3,
  named: [commit('xxxxxxxx', 'cccc3333', '2026-11-09T10:00:00Z', 'lanes: landed', ['master'], true)],
}
const GIT: Workspace = {
  root: '/g',
  vcs: 'git',
  head: commit(undefined, '1234567', '2026-11-09T09:00:00Z', 'g2', ['main'], false),
  changed: 2,
  line: [],
  more: 0,
  named: [],
}

const ITEM: Item = {
  number: 7,
  title: 'make the thing',
  url: 'https://w/7',
  updated: '2026-11-09T10:00:00Z',
  labels: ['owner/lane-a'],
  body: 'his words',
  newest: { by: 'dispatch', at: '2026-11-09T11:00:00Z', body: 'half done' },
}

const transcript = (id: string) => `${HOME}/.claude/projects/-w/${id}.jsonl`
/** Where a background job's transcript goes once it moves into a worktree, while its hooks still name the first path. */
const MOVED = `${HOME}/.claude/projects/-w--claude-worktrees-x/s1.jsonl`
const heard = (...kept: Heard['kept']): Heard => ({ kept, older: 0 })
/** What hooks/messages.jq prints for each transcript; tests/messages.test.sh tests the walk and the bounds. */
const SAID: Record<string, { owen: Heard; peers: Heard }> = {
  [transcript('s1')]: { owen: heard(), peers: heard({ at: '2026-11-09T12:34:56Z', from: 'dispatch', text: 'dispatch → lane-a: go', more: 0 }) },
  [transcript('s2')]: { owen: heard({ at: '2026-11-09T13:00:00Z', from: 'Owen', text: 'said after the clear', more: 0 }), peers: heard() },
  [MOVED]: { owen: heard({ at: '2026-11-09T12:40:00Z', from: 'Owen', text: 'said in the worktree', more: 0 }), peers: heard() },
}
/** messages.jq stops on an undefined $caps, so a call without them fails as it would on the host. */
const passesCaps = (argv: readonly string[]) => argv.some((arg, i) => arg === '--argjson' && argv[i + 1] === 'caps')

const ran = (stdout: string, exitCode = 0, stderr = ''): ProcessRunResult => ({ exitCode, stdout, stderr, isStdoutTruncated: false, isStderrTruncated: false })

type World = {
  named?: boolean
  linked?: boolean
  tokens?: number
  items?: ProcessRunResult
  jq?: ProcessRunResult
  commits?: ProcessRunResult
  clears?: boolean
  ab?: string
  refusedRegisters?: number
  check?: ProcessRunResult
  exe?: ProcessRunResult
  /** What hooks/mine.sh prints for each transcript: the ids of the comments its session posted, and the issues it filed. */
  posted?: Record<string, number[]>
  filed?: Record<string, number[]>
  /** What hooks/open.sh prints: those of the issues it is asked about that are open; read at each call. */
  open?: { now: { number: number; title: string; comments: number; closes: string | null }[] }
}

/** The engine and file system beneath the mod: the session s1 named lane-a (`named`), its state file linked (`linked`), a
 * `/clear` that moves it to s2 (`clears`), running the clear's SessionStart hooks before it resolves, an A/B trial
 * started (`ab`, the file's text), and an engine that refuses the first `refusedRegisters` tool registrations, as 2.1.322
 * does before a session is bound to the process. */
function world(
  $: Engine,
  on: On,
  { named = true, linked = true, tokens = 250_000, items = ran(JSON.stringify([ITEM])), jq, commits = ran(JSON.stringify([JJ])), clears = true, ab, refusedRegisters = 0, check = ran(''), exe = ran(`${CLAUDE_BIN}\n`), posted = {}, filed = {}, open = { now: [] } }: World = {},
) {
  const files: Record<string, string> = {
    [`${HOME}/.claude/sessions/41.json`]: JSON.stringify({ pid: 41, sessionId: 's1', ...(named && { name: 'lane-a' }) }),
    [`${HOME}/.claude/sessions/42.json`]: JSON.stringify({ pid: 42, sessionId: 'other', name: 'lane-b' }),
    [STATE]: STATE_TEXT,
    [NOTES]: 'carried notes',
    [transcript('s1')]: '',
    [transcript('s2')]: '',
    [`${MAIL}/s1.json`]: '{"topics":["harness"]}',
    ...(ab !== undefined && { [AB]: ab }),
  }
  const seen = {
    id: 's1',
    usage: { tokens, window: 1_000_000 },
    commands: [] as string[],
    prompts: [] as { text: string; context?: readonly string[] }[],
    tools: [] as string[],
    logs: [] as string[],
    runs: [] as (readonly string[])[],
  }
  mock.env(on, { HOME })
  const rows = mock.session(on)
  const clock = mock.clock(on, { now: 0 })
  const mtimes: Record<string, number> = {}
  on('fs.read', (_$, e) => (e.path in files ? { value: files[e.path] } : { deny: `ENOENT: ${e.path}` }))
  on('fs.write', (_$, e) => ((files[e.path] = e.text), { value: undefined }))
  on('fs.exists', (_$, e) => ({ value: e.path in files }))
  on('fs.list', (_$, e) => {
    const below = Object.keys(files).flatMap((path) => (path.startsWith(`${e.path}/`) ? [path.slice(e.path.length + 1).split('/')] : []))
    const kinds = new Map(below.map(([name, ...deeper]) => [name, deeper.length === 0 ? ('file' as const) : ('dir' as const)]))
    return { value: [...kinds].map(([name, kind]) => ({ name, kind, size: 0, mtimeMs: 0, isLink: false })) }
  })
  on('fs.stat', (_$, e) =>
    e.path === LINK && linked
      ? { value: { kind: 'file', size: 0, mtimeMs: 0, isLink: true, realPath: STATE } }
      : e.path in files
        ? { value: { kind: 'file', size: files[e.path].length, mtimeMs: mtimes[e.path] ?? 0, isLink: false } }
        : { deny: `ENOENT: ${e.path}` },
  )
  on('process.run', (_$, e) => {
    seen.runs.push(e.argv)
    return {
      value:
        e.argv[0] === 'setsid'
          ? check
          : e.argv[0] === 'readlink'
          ? e.argv.join(' ') === 'readlink -f /proc/41/exe' ? exe : ran('', 1, `readlink: ${e.argv.at(-1)}: No such file or directory`)
          : e.argv[1]?.endsWith('/hooks/open.sh')
          ? ran(JSON.stringify(open.now.filter(({ number }) => e.argv.slice(3).includes(String(number)))))
          : e.argv[1]?.endsWith('/hooks/mine.sh')
          ? e.argv[3] === 'agents/work'
            ? ran(JSON.stringify({ comments: posted[e.argv[2]] ?? [], issues: filed[e.argv[2]] ?? [] }))
            : ran('', 2, 'mine.sh: no such repository')
          : e.argv[0] !== 'jq'
          ? e.argv[1]?.endsWith('/hooks/commits.sh')
            ? commits
            : items
          : !passesCaps(e.argv)
            ? ran('', 3, 'jq: error: $caps is not defined')
            : (jq ?? ran(JSON.stringify(SAID[e.argv.at(-1) as string] ?? { owen: heard(), peers: heard() }))),
    }
  })
  on('session.id', () => ({ value: seen.id }))
  on('session.cwd', () => ({ value: '/w' }))
  on('tool.describe', (_$, e) => ({ description: e.description, ...(e.isDeferred && { isDeferred: true }) }))
  on('session.usage', () => ({ value: { startedAt: 0, context: seen.usage, rateLimits: [] } }))
  on('session.start', (_$, e) => ({ cwd: e.cwd }))
  let refusals = refusedRegisters
  on('tool.register', (_$, e) =>
    refusals-- > 0
      ? { deny: '$.tool.register is not available in this mode: no session is bound in this process' }
      : (seen.tools.push(e.name), { value: { tool: `mcp__lane__${e.name}` } }),
  )
  on('turn.complete', (_$, e) => ({ text: e.answer }))
  on('ui.toast', (_$, e) => (seen.logs.push(e.text), { value: undefined }))
  on('ui.log', (_$, e) => (seen.logs.push(e.text), { value: undefined }))
  on('classic.SessionStart', () => ({}))
  on('classic.Stop', () => ({}))
  on('classic.PostToolUse', () => ({}))
  on('prompt.submit', (_$, e) => (seen.prompts.push({ text: e.text, context: e.context }), { text: e.text, context: e.context }))
  on('command.run', async (_$, e) => {
    seen.commands.push(e.command)
    if (e.command === 'clear' && clears) {
      seen.id = 's2'
      files[`${HOME}/.claude/sessions/41.json`] = files[`${HOME}/.claude/sessions/41.json`].replace('"s1"', '"s2"')
      await sessionStart($, 'clear', 's2')
    }
    return {}
  })
  const appended = () => rows.appended().map(({ message }) => message.content.map((block) => (block.type === 'text' ? block.text : `[${block.type}]`)))
  return { seen, files, clock, mtimes, appended }
}

const sessionStart = ($: Engine, source: 'startup' | 'clear' | 'compact' | 'resume', id = 's1') =>
  $.classic.SessionStart({ hook_event_name: 'SessionStart', source, session_id: id, transcript_path: transcript(id), cwd: '/w' })
const start = async ($: Engine) => {
  await $.session.start({ cwd: '/w', surface: 'terminal', isInteractive: true })
  await sessionStart($, 'startup')
}
const prompt = ($: Engine, text: string) => $.prompt.submit({ text, origin: { kind: 'composer' }, wait: false })
const bash = ($: Engine, command: string) =>
  $.classic.PostToolUse({ hook_event_name: 'PostToolUse', session_id: 's1', transcript_path: transcript('s1'), cwd: '/w', tool_name: 'Bash', tool_input: { command }, tool_response: {}, tool_use_id: 't' })
const shell = (command: string): BackgroundTaskSummary => ({ id: 'b1', type: 'shell', status: 'running', description: command, command })
const stop = ($: Engine, tasks: BackgroundTaskSummary[] = [], { active = false, crons = [] as SessionCronSummary[] } = {}) =>
  $.classic.Stop({ hook_event_name: 'Stop', session_id: 's1', transcript_path: transcript('s1'), cwd: '/w', stop_hook_active: active, background_tasks: tasks, session_crons: crons })
const end = ($: Engine, extra = {}) => $.turn.complete({ reason: 'answer', answer: 'done', durationMs: 1, isAborted: false, turnId: 't1', ...extra })
const rotateCall = ($: Engine, extra = {}) => $.tool.call({ tool: TOOL, tool_use_id: 'r1', ...extra } as never)
const describe = ($: Engine, tool: string) => $.tool.describe({ tool, description: 'd', isDeferred: true, provider: { plugin: 'engine', tier: 'core' } } as never)

test("the first prompt after a start carries the state file, what it carries, the lane's open issues and the mail it watches", async ($, on) => {
  const { seen } = world($, on)
  await start($)
  await prompt($, 'hello')
  const [state, notes, gone, items, mail] = seen.prompts[0].context ?? []
  expect(state).toContain(`Your state file is ${STATE} (${LINK} names it)`)
  expect(state).toContain('## Goal\n- ship it')
  expect(notes).toContain(`${NOTES}, which the state file carries, as it read when this context began:\n\ncarried notes`)
  expect(gone).toContain('/w/gone.md, which the state file carries, could not be read')
  expect(items).toContain('### #7 make the thing\nhttps://w/7')
  expect(items).toContain('Newest comment, by dispatch at 2026-11-09T11:00:00Z:\nhalf done')
  expect(mail).toBe(`This session watches these agent-mail topics and threads (its watch record, ${MAIL}/s1.json):\n- topics: harness\n- threads: none`)
  expect(seen.prompts[0].context).toHaveLength(5)
})

test("a named session's most used deferred tools are listed in front, and the others stay deferred", async ($, on) => {
  world($, on)
  expect(await describe($, 'SendMessage')).toEqual({ description: 'd', isDeferred: false })
  expect(await describe($, 'NotebookEdit')).toEqual({ description: 'd', isDeferred: true })
})

test('an unnamed session is left alone: no rotate tool, no tools moved, no kit, no mail copied at a clear', async ($, on) => {
  const { seen, files } = world($, on, { named: false })
  await start($)
  expect(seen.tools).toEqual([])
  expect(await describe($, 'SendMessage')).toEqual({ description: 'd', isDeferred: true })
  await prompt($, 'hello')
  expect(seen.prompts[0].context).toBeUndefined()
  await sessionStart($, 'clear', 's2')
  expect(`${MAIL}/s2.json` in files).toBe(false)
})

test('only the first prompt carries it', async ($, on) => {
  const { seen } = world($, on)
  await start($)
  await prompt($, 'one')
  await prompt($, 'two')
  expect(seen.prompts[1].context).toBeUndefined()
})

test('a resume into a session with no name drops the lane it had', async ($, on) => {
  const { seen } = world($, on)
  await start($)
  await prompt($, 'one')
  seen.id = 'unnamed'
  await sessionStart($, 'resume', 'unnamed')
  await prompt($, 'two')
  expect(seen.prompts[1].context).toBeUndefined()
})

test('a lane with no state file is told to make one', async ($, on) => {
  const { seen } = world($, on, { linked: false })
  await start($)
  await prompt($, 'hello')
  expect(seen.prompts[0].context?.[0]).toContain('has no state file')
  expect(seen.prompts[0].context?.[0]).toContain('`just lane-state lane-a`')
})

test('a tracker that cannot be read is named, and the rest still loads', async ($, on) => {
  const { seen } = world($, on, { items: ran('', 22, 'curl: (22) The requested URL returned error: 401') })
  await start($)
  await prompt($, 'hello')
  expect(seen.prompts[0].context?.[3]).toBe(
    'The open agents/work issues given to lane-a could not be read (curl: (22) The requested URL returned error: 401). List them yourself before you pick up work.',
  )
})

test('past the unit mark, a turn that landed work and left nothing running but a mail wait is asked to rotate', async ($, on) => {
  world($, on)
  await start($)
  await bash($, 'cd ~/tmp/w && ~/workspace/claude/tools/land "notes: x" lanes/lane-a.md')
  const asked = await stop($, [shell('timeout 600 agent-mail wait --topic harness')])
  expect(asked.block).toContain('You landed work this turn and the context holds 250k tokens')
  expect(asked.block).toContain('Then call the rotate tool')
})

test('a command that lands work past the unit mark is asked at once, with its result, before the turn goes on', async ($, on) => {
  world($, on)
  await start($)
  const [asked] = (await bash($, 'jj git push --bookmark x')).additionalContext ?? []
  expect(asked).toContain('You landed work this turn and the context holds 250k tokens')
  expect(asked).toContain('If anything you started still runs, say in the state file what each is for')
  expect(asked).toContain('Then call the rotate tool and end your turn.')
})

test('a landing is not asked under the unit mark, from a subagent, or again until the context grows', async ($, on) => {
  const { seen } = world($, on, { tokens: 150_000 })
  await start($)
  expect((await bash($, 'just land "notes: x" a.md')).additionalContext).toBeUndefined()
  seen.usage.tokens = 250_000
  const fromAgent = await $.classic.PostToolUse({ hook_event_name: 'PostToolUse', session_id: 's1', transcript_path: transcript('s1'), cwd: '/w', agent_id: 'a1', agent_type: 'general-purpose', tool_name: 'Bash', tool_input: { command: 'git push' }, tool_response: {}, tool_use_id: 't' } as never)
  expect(fromAgent.additionalContext).toBeUndefined()
  expect((await bash($, 'git push')).additionalContext).toHaveLength(1)
  seen.usage.tokens = 300_000
  expect((await bash($, 'git push')).additionalContext).toBeUndefined()
  seen.usage.tokens = 350_000
  expect((await bash($, 'git push')).additionalContext?.[0]).toContain('350k')
})

test('a turn that landed nothing, or left work running, is not asked under the mark', async ($, on) => {
  world($, on)
  await start($)
  expect((await stop($)).block).toBeUndefined()
  await bash($, 'jj git push --bookmark x')
  expect((await stop($, [shell('cargo test')])).block).toBeUndefined()
})

test('under the unit floor a landed turn is not asked', async ($, on) => {
  world($, on, { tokens: 150_000 })
  await start($)
  await bash($, 'just land "notes: x" a.md')
  expect((await stop($)).block).toBeUndefined()
})

test('past the mark any turn is asked, naming what still runs, and once until the context grows', async ($, on) => {
  const { seen } = world($, on, { tokens: 650_000 })
  await start($)
  const asked = await stop($, [shell('cargo test')])
  expect(asked.block).toContain('The context holds 650k tokens')
  expect(asked.block).toContain('Still running: cargo test')
  seen.usage.tokens = 700_000
  expect((await stop($)).block).toBeUndefined()
  seen.usage.tokens = 760_000
  expect((await stop($)).block).toContain('760k')
})

test('a Stop the ask itself continued is not asked again', async ($, on) => {
  world($, on, { tokens: 650_000 })
  await start($)
  expect((await stop($, [], { active: true })).block).toBeUndefined()
})

test('with no SessionStart seen, as after a reload of the mod, a Stop names the session and the ask still comes', async ($, on) => {
  world($, on, { tokens: 650_000 })
  expect((await stop($)).block).toContain('The context holds 650k tokens')
})

test('rotate clears the context once the turn ends, hands the fresh one the kit and what the cleared one was sent, then has it carry on', async ($, on) => {
  const { seen, files, clock, appended } = world($, on)
  await start($)
  await prompt($, 'hello')
  expect(await rotateCall($)).toEqual({ result: 'the context starts over when this turn ends; end the turn now' })
  expect(seen.commands).toEqual([])
  await end($)
  await clock.advance(500)
  expect(seen.commands).toEqual(['clear'])
  const [kit] = appended()
  expect(kit[0]).toContain(`Your state file is ${STATE}`)
  expect(kit.at(-3)).toContain(`(its watch record, ${MAIL}/s2.json):\n- topics: harness`)
  expect(kit.at(-2)).toContain('From other sessions:\n\n[2026-11-09 12:34Z, dispatch]\ndispatch → lane-a: go')
  expect(kit.at(-2)).not.toContain('said after the clear')
  expect(kit.at(-1)).toBe('Running in the background when the last turn ended (anything started since is not listed):\n- nothing')
  expect(seen.prompts.at(-1)).toEqual({ text: CARRY_ON })
  expect(files[`${MAIL}/s2.json`]).toBe('{"topics":["harness"]}')
  await prompt($, 'next')
  expect(seen.prompts.at(-1)?.context).toBeUndefined()
})

test('a background job that moved into a worktree has the messages it was sent read from where its transcript went', async ($, on) => {
  const { files, clock, appended } = world($, on)
  delete files[transcript('s1')]
  files[MOVED] = ''
  await start($)
  await prompt($, 'hello')
  await rotateCall($)
  await end($)
  await clock.advance(500)
  const [kit] = appended()
  expect(kit.at(-2)).toContain('From Owen:\n\n[2026-11-09 12:40Z, Owen]\nsaid in the worktree')
})

test('a /clear that leaves the session as it was is reported to it, with no kit', async ($, on) => {
  const { seen, clock, appended } = world($, on, { clears: false })
  await start($)
  await rotateCall($)
  await end($)
  await clock.advance(500)
  expect(appended()).toEqual([])
  expect(seen.prompts.at(-1)?.text).toBe('The rotation did not happen (/clear ran, but the session kept its id). This context carries on as it was.')
})

test('a /clear he runs himself hands his next prompt the kit and what the cleared context was sent, and copies the mail subscriptions', async ($, on) => {
  const { seen, files } = world($, on)
  await start($)
  await prompt($, 'hello')
  seen.id = 's2'
  await sessionStart($, 'clear', 's2')
  await prompt($, 'after the clear')
  expect(seen.prompts.at(-1)?.context?.at(-2)).toContain('dispatch → lane-a: go')
  expect(files[`${MAIL}/s2.json`]).toBe('{"topics":["harness"]}')
})

test('rotate is refused to a subagent and to a lane with no state file', async ($, on) => {
  world($, on, { linked: false })
  await start($)
  expect(await rotateCall($, { agentId: 'a1' })).toEqual({ deny: 'only the main conversation starts its context over' })
  expect(await rotateCall($)).toEqual({ deny: `${LINK} leads nowhere: make the state file first (\`just lane-state lane-a\`)` })
})

test('rotate is refused while the state file is as it was before the ask, since the next context starts from it', async ($, on) => {
  const { clock, mtimes } = world($, on)
  await start($)
  await clock.advance(1_000)
  await bash($, 'git push')
  expect((await rotateCall($)).deny).toBe(
    `${STATE} has not changed since you were asked to start over, and it is what the next context starts from: update it first. If writing it is refused, say so and end your turn without rotating.`,
  )
  mtimes[STATE] = 2_000
  expect(await rotateCall($)).toEqual({ result: 'the context starts over when this turn ends; end the turn now' })
})

test('an interrupted turn drops the rotation', async ($, on) => {
  const { seen, clock } = world($, on)
  await start($)
  await rotateCall($)
  await end($, { reason: 'aborted', isAborted: true })
  await end($)
  await clock.advance(10_000)
  expect(seen.commands).toEqual([])
})

const OLD: SessionMessage = { role: 'user', text: 'old', toolUses: [], handle: 'h0' }
const SUMMARY: SessionMessage = { role: 'user', text: 'the summary', toolUses: [], handle: 'h1' }
const compactions = (on: On) => on('session.compact', (): SessionCompactResult => ({ messages: [SUMMARY], tokensBefore: 900_000, tokensAfter: 4_000 }))

test('a compaction keeps what was sent, the kit and what ran before the summary, and the transcript after it', async ($, on) => {
  world($, on)
  compactions(on)
  await start($)
  await stop($, [shell('cargo test')])
  const compacted = await $.session.compact({ trigger: 'auto', messages: [OLD] })
  const texts = compacted.messages?.map(({ text }) => text) ?? []
  expect(texts[0]).toContain(`Your state file is ${STATE}`)
  expect(texts.at(-4)).toContain('dispatch → lane-a: go')
  expect(texts.at(-3)).toBe('Running in the background when the last turn ended (anything started since is not listed):\n- shell: cargo test')
  expect(texts.at(-2)).toBe('the summary')
  expect(texts.at(-1)).toBe(`This session's whole transcript, from before every compaction, is ${transcript('s1')}; recall searches it.`)
  expect(compacted.tokensAfter).toBeUndefined()
})

test('a compaction names the crons set to wake the session, since the summary may not', async ($, on) => {
  world($, on)
  compactions(on)
  await start($)
  const crons: SessionCronSummary[] = [
    { id: 'c1', schedule: '*/30 * * * *', recurring: true, prompt: 'check the build' },
    { id: 'c2', schedule: '15 7 10 11 *', recurring: false, prompt: 'mail Owen the page' },
  ]
  await stop($, [shell('cargo test')], { crons })
  const compacted = await $.session.compact({ trigger: 'auto', messages: [OLD] })
  expect(compacted.messages?.at(-3)?.text).toBe(
    'Running in the background when the last turn ended (anything started since is not listed):\n- shell: cargo test\n\n' +
      'Set to wake this session then, each with the prompt it submits (CronList shows which are still set):\n' +
      '- `*/30 * * * *`: check the build\n- `15 7 10 11 *` (once): mail Owen the page',
  )
})

test('a compaction whose messages cannot be read still carries the rest, and says so', async ($, on) => {
  world($, on, { jq: ran('', 134, 'jq: error: cannot allocate memory') })
  compactions(on)
  await start($)
  const texts = (await $.session.compact({ trigger: 'auto', messages: [OLD] })).messages?.map(({ text }) => text) ?? []
  expect(texts[0]).toContain(`Your state file is ${STATE}`)
  expect(texts.at(-4)).toBe(`The messages this session was sent could not be read (messages.jq exited 134: jq: error: cannot allocate memory); they are in ${transcript('s1')}.`)
})

/** The recall checks started, the mod's own directory left out of the script's path. */
const checksRun = (runs: (readonly string[])[]) =>
  runs.filter(([command]) => command === 'setsid').map(([setsid, detach, shell, script, ...rest]) => [setsid, detach, shell, script.split('/').slice(-2).join('/'), ...rest])
const rotation = (files: Record<string, string>, dir: string) => JSON.parse(files[`${dir}/rotation.json`])

test('a rotation has its recall check started apart from the session, with the kit it handed over and where its notes are', async ($, on) => {
  const { seen, files, clock, mtimes, appended } = world($, on)
  await start($)
  await clock.advance(1_000)
  await bash($, 'git push')
  mtimes[STATE] = 2_000
  await rotateCall($)
  await end($)
  await clock.advance(500)
  const dir = `${CHECKS}/lane-a/s2`
  expect(checksRun(seen.runs)).toEqual([['setsid', '-f', 'bash', 'hooks/check.sh', dir]])
  expect(files[`${dir}/kit.txt`]).toBe(appended()[0].join('\n\n'))
  expect(rotation(files, dir)).toEqual({
    lane: 'lane-a',
    arm: 'A',
    from: 's1',
    to: 's2',
    transcript: transcript('s1'),
    notes: [LINK, STATE],
    tracker: 'agents/work',
    at: '1970-01-01T00:00:01.500Z',
    claude: CLAUDE_BIN,
    results: `${CHECKS}/lane-a.jsonl`,
  })
  expect(seen.prompts.at(-1)).toEqual({ text: CARRY_ON })
  expect(seen.runs.filter((argv) => argv[1]?.endsWith('/hooks/items.sh')).map((argv) => argv.slice(2))).toContainEqual(['lane-a', 'agents/work'])
})

test("in arm B the check is told the arm, a lane with no state file only where its link would be, and an engine whose binary isn't found none", async ($, on) => {
  const { files, clock } = world($, on, { ab: B_FIRST, linked: false, exe: ran('', 1, 'readlink: /proc/41/exe: No such file or directory') })
  await start($)
  await prompt($, 'hello')
  await rotateCall($)
  await end($)
  await clock.advance(500)
  const { arm, notes, claude } = rotation(files, `${CHECKS}/lane-a/s2`)
  expect([arm, notes, claude]).toEqual(['B', [LINK], undefined])
})

test('a /clear he runs himself and a compaction start no check: they are no rotation', async ($, on) => {
  const { seen } = world($, on)
  compactions(on)
  await start($)
  await prompt($, 'hello')
  await $.session.compact({ trigger: 'auto', messages: [OLD] })
  seen.id = 's2'
  await sessionStart($, 'clear', 's2')
  await prompt($, 'after the clear')
  expect(checksRun(seen.runs)).toEqual([])
})

test('a check that does not start is logged, and the fresh context carries on all the same', async ($, on) => {
  const { seen, clock } = world($, on, { check: ran('', 1, 'setsid: failed to execute bash: No such file or directory') })
  await start($)
  await prompt($, 'hello')
  await rotateCall($)
  await end($)
  await clock.advance(500)
  expect(seen.logs).toContain('lane: the recall check of this rotation did not start: setsid: failed to execute bash: No such file or directory')
  expect(seen.prompts.at(-1)).toEqual({ text: CARRY_ON })
})

test("an issue the lane commented on shows the newest comment by anyone else and the lane's own newest, each with its author and time", () => {
  const ordered = { by: 'agent', at: '2026-11-09T11:00:00Z', body: 'lane-a: start with the step in your first prompt.' }
  const note = { by: 'agent', at: '2026-11-09T12:00:00Z', body: 'state as of df4ed5f: read 1-5000 logged and pushed' }
  const block = itemsBlock('lane-a', [{ ...ITEM, newest: ordered, mine: note }, { ...ITEM, number: 8, newest: null, mine: note }, { ...ITEM, number: 9 }])
  expect(block).toContain(`Newest comment by anyone else, by agent at 2026-11-09T11:00:00Z:\n${ordered.body}\n\nNewest comment by this lane, by agent at 2026-11-09T12:00:00Z:\n${note.body}`)
  expect(block).toContain(`No comment by anyone else.\n\nNewest comment by this lane, by agent at 2026-11-09T12:00:00Z:\n${note.body}`)
  expect(block).toContain('### #9 make the thing\nhttps://w/7 · updated 2026-11-09T10:00:00Z · owner/lane-a\n\nhis words\n\nNewest comment, by dispatch at 2026-11-09T11:00:00Z:\nhalf done')
  expect(block).toContain('most recently updated first, each with its newest comment by anyone else and, where this lane commented, its own newest:')
})

const itemsRun = (runs: (readonly string[])[]) => runs.filter((argv) => argv[1]?.endsWith('/hooks/items.sh')).map((argv) => argv.slice(2))
const postedRun = (runs: (readonly string[])[]) => runs.filter((argv) => argv[1]?.endsWith('/hooks/mine.sh')).map((argv) => argv[2])
const contexts = (files: Record<string, string>) => files[CONTEXTS].trim().split('\n').map((line) => JSON.parse(line))

test("in arm B the comments the lane posted in any of its contexts are told from everyone else's", async ($, on) => {
  const ordered = { by: 'agent', at: '2026-11-09T11:00:00Z', body: 'lane-a: start with the step in your first prompt.' }
  const note = { by: 'agent', at: '2026-11-09T12:00:00Z', body: 'state as of df4ed5f: read 1-5000 logged and pushed' }
  const items = ran(JSON.stringify([{ ...ITEM, newest: ordered, mine: note }]))
  const { seen, files, clock, appended } = world($, on, { ab: B_FIRST, linked: false, items, posted: { [transcript('s1')]: [52] } })
  files[CONTEXTS] = `${JSON.stringify({ id: 's0', transcript: transcript('s0'), comments: [40] })}\n`
  await start($)
  await prompt($, 'hello')
  expect(itemsRun(seen.runs).at(-1)).toEqual(['lane-a', 'agents/work', '[40]'])
  await rotateCall($)
  await end($)
  await clock.advance(500)
  expect(itemsRun(seen.runs).at(-1)).toEqual(['lane-a', 'agents/work', '[40,52]'])
  expect(contexts(files)).toEqual([
    { id: 's0', transcript: transcript('s0'), comments: [40] },
    { id: 's1', transcript: transcript('s1'), comments: [52], issues: [] },
    { id: 's2', transcript: transcript('s2') },
  ])
  const [kit] = appended()
  expect(kit[0]).toContain(`Newest comment by anyone else, by agent at 2026-11-09T11:00:00Z:\n${ordered.body}`)
  expect(kit[0]).toContain(`Newest comment by this lane, by agent at 2026-11-09T12:00:00Z:\n${note.body}`)
})

test('in arm B an issue the lane filed in any of its contexts is to be closed once its "Closes with" holds; one filed by others is not', async ($, on) => {
  const items = ran(JSON.stringify([{ ...ITEM, number: 42 }, { ...ITEM, number: 9 }, ITEM]))
  const { clock, files, appended } = world($, on, { ab: B_FIRST, linked: false, items, filed: { [transcript('s1')]: [42] } })
  files[CONTEXTS] = `${JSON.stringify({ id: 's0', transcript: transcript('s0'), comments: [], issues: [7] })}\n`
  await start($)
  await prompt($, 'hello')
  await rotateCall($)
  await end($)
  await clock.advance(500)
  const [kit] = appended()
  const close = 'You filed this issue: once its "Closes with" holds, close it, with a comment saying what met it.'
  const issue = (n: number) => kit[0].split('### #').find((part) => part.startsWith(`${n} `)) ?? ''
  expect(issue(42)).toContain(close)
  expect(issue(7)).toContain(close)
  expect(issue(9)).not.toContain(close)
  expect(contexts(files)[1]).toEqual({ id: 's1', transcript: transcript('s1'), comments: [], issues: [42] })
})

test('a context that has ended is read for its comments once, and one a compaction keeps going is read again each time', async ($, on) => {
  const { seen, clock } = world($, on, { ab: B_FIRST, linked: false, posted: { [transcript('s1')]: [52], [transcript('s2')]: [60] } })
  compactions(on)
  await start($)
  await prompt($, 'hello')
  await rotateCall($)
  await end($)
  await clock.advance(500)
  await $.session.compact({ trigger: 'auto', messages: [OLD] })
  await $.session.compact({ trigger: 'auto', messages: [OLD] })
  expect(postedRun(seen.runs)).toEqual([transcript('s1'), transcript('s2'), transcript('s2')])
  expect(itemsRun(seen.runs).at(-1)).toEqual(['lane-a', 'agents/work', '[52,60]'])
})

test('in arm A each context is still recorded, for a later switch, but no comment is read', async ($, on) => {
  const { seen, files, clock } = world($, on)
  await start($)
  await prompt($, 'hello')
  await rotateCall($)
  await end($)
  await clock.advance(500)
  expect(contexts(files)).toEqual([{ id: 's1', transcript: transcript('s1') }, { id: 's2', transcript: transcript('s2') }])
  expect(postedRun(seen.runs)).toEqual([])
  expect(itemsRun(seen.runs).at(-1)).toEqual(['lane-a', 'agents/work'])
})

test("a log of contexts that can't be read is reported, and the issues still load, without the lane's own comments told apart", async ($, on) => {
  const { seen, files } = world($, on, { ab: B_FIRST, linked: false })
  files[CONTEXTS] = '{"id": "s0", torn\n'
  await start($)
  await prompt($, 'hello')
  expect(seen.logs.some((line) => line.startsWith(`lane: ${CONTEXTS} was not read, so this kit tells no comment as the lane's own:`))).toBe(true)
  expect(itemsRun(seen.runs).at(-1)).toEqual(['lane-a', 'agents/work'])
  expect(seen.prompts[0].context?.[0]).toContain('### #7 make the thing')
})

const openRun = (runs: (readonly string[])[]) => runs.filter((argv) => argv[1]?.endsWith('/hooks/open.sh')).map((argv) => argv.slice(2))
const FILED = `${JSON.stringify({ id: 's0', transcript: transcript('s0'), comments: [], issues: [42, 43, 44] })}\n`
const closing = (...issues: string[]) =>
  `These issues you filed are still open:\n${issues.join('\n')}\nFor each: if its "Closes with" holds now, close it with a comment saying what met it; if not, carry on.`

test("at a turn's end in arm B the open issues the lane filed are put before it, each with what closes it, in one continuation", async ($, on) => {
  const open = { now: [{ number: 42, title: 'hold the count', comments: 1, closes: 'the user has asked for the count and been given it.' }, { number: 43, title: 'fix the build', comments: 0, closes: null }] }
  const { seen, files } = world($, on, { ab: B_FIRST, linked: false, open })
  files[CONTEXTS] = FILED
  await start($)
  await prompt($, 'what was the count?')
  expect((await stop($)).block).toBe(closing('- #42 hold the count. Closes with: the user has asked for the count and been given it.', '- #43 fix the build. It has no "Closes with" line: write one.'))
  expect(openRun(seen.runs)).toEqual([['agents/work', '42', '43', '44']])
  expect((await stop($, [], { active: true })).block).toBeUndefined()
})

test("an issue whose condition doesn't hold is put before the lane once a user turn, again in it only after a new comment, and not for a notification's turn", async ($, on) => {
  const open = { now: [{ number: 42, title: 'hold the count', comments: 1, closes: 'the user has asked for the count.' }] }
  const { files } = world($, on, { ab: B_FIRST, linked: false, open })
  files[CONTEXTS] = FILED
  await start($)
  await prompt($, 'one')
  const asked = closing('- #42 hold the count. Closes with: the user has asked for the count.')
  expect((await stop($)).block).toBe(asked)
  expect((await stop($)).block).toBeUndefined()
  open.now[0].comments = 2
  expect((await stop($)).block).toBe(asked)
  expect((await stop($)).block).toBeUndefined()
  await $.prompt.submit({ text: 'a background task finished', origin: { kind: 'task-notification' }, wait: false })
  expect((await stop($)).block).toBeUndefined()
  await prompt($, 'two')
  expect((await stop($)).block).toBe(asked)
})

test('the ask to start over comes before the issues, and arm A is never asked to close any', async ($, on) => {
  const open = { now: [{ number: 42, title: 'hold the count', comments: 1, closes: 'x' }] }
  const b = world($, on, { ab: B_FIRST, linked: false, open, tokens: 650_000 })
  b.files[CONTEXTS] = FILED
  await start($)
  expect((await stop($)).block).toContain('The context holds 650k tokens')
  expect(openRun(b.seen.runs)).toEqual([])
})

test('in arm A no issue is put before the lane at a turn\'s end', async ($, on) => {
  const { seen, files } = world($, on, { open: { now: [{ number: 42, title: 'hold the count', comments: 1, closes: 'x' }] } })
  files[CONTEXTS] = FILED
  await start($)
  expect((await stop($)).block).toBeUndefined()
  expect(openRun(seen.runs)).toEqual([])
})

test('front matter names the files a state file carries', () => {
  expect(carried(STATE_TEXT)).toEqual([NOTES, '/w/gone.md'])
  expect(carried('# no front matter\ncarry:\n  - /x\n')).toEqual([])
})

test('a state file over the cap is cut at a line, or at the cap when no line ends near it, saying so', () => {
  const lines = 'a line\n'.repeat(CAPS.state / 7 + 10)
  expect(stateBlock(LINK, STATE, lines)).toContain(`[Cut here: the file is ${lines.length} characters`)
  expect(stateBlock(LINK, STATE, lines).length).toBeLessThan(CAPS.state + 600)
  expect(stateBlock(LINK, STATE, 'x'.repeat(200_000)).length).toBeLessThan(CAPS.state + 600)
})

test('a carried file cut by the cap says so, however little is cut', () => {
  const [, second] = carriedBlocks([
    { path: '/a', text: 'a'.repeat(CAPS.carried - 3) },
    { path: '/b', text: 'bbbbbb' },
  ])
  expect(second).toContain('(cut: read the rest with Read)')
})

test('issues past the cap are listed by title alone, the least recently updated', () => {
  const big = (number: number): Item => ({ ...ITEM, number, body: 'x'.repeat(CAPS.itemText) })
  const block = itemsBlock('lane-a', Array.from({ length: 20 }, (_, i) => big(100 - i)))
  expect(block).toContain('### #100 make the thing')
  expect(block).not.toContain('### #81 ')
  expect(block.endsWith('\n- #81 make the thing https://w/7')).toBe(true)
  expect(block).toContain('Also open, less recently updated, without their text:\n- #')
})

test("one long report from another session pushes out neither Owen's messages nor the rest", () => {
  const block = saidBlock(
    {
      owen: heard({ at: '2026-11-09T10:00:00Z', from: 'Owen', text: 'please do X', more: 0 }),
      peers: heard(
        { at: '2026-11-09T10:01:00Z', from: 'dispatch', text: 'short note', more: 0 },
        { at: '2026-11-09T10:02:00Z', from: 'adversary', text: 'r'.repeat(CAPS.message), more: 19_000 },
      ),
    },
    '/t.jsonl',
  )
  expect(block).toContain('From Owen:\n\n[2026-11-09 10:00Z, Owen]\nplease do X')
  expect(block).toContain('[2026-11-09 10:01Z, dispatch]\nshort note')
  expect(block).toContain(`${'r'.repeat(CAPS.message)}… [19000 more characters]`)
})

test('the messages messages.jq left out count among the older ones', () => {
  const block = saidBlock({ owen: { kept: [], older: 700 }, peers: heard() }, '/t.jsonl')
  expect(block).toContain('From Owen:\n\n(none)\n\n[700 older ones are in /t.jsonl.]')
})

test('the titles of issues past the cap stay within it too, and the rest are counted', () => {
  const big = (number: number): Item => ({ ...ITEM, number, body: 'x'.repeat(CAPS.itemText) })
  const block = itemsBlock('lane-a', Array.from({ length: 1_000 }, (_, i) => big(5_000 - i)))
  expect(block.length).toBeLessThan(CAPS.items + 2_000)
  expect(block).toMatch(/\n\n\[\d+ more are open, the least recently updated; list them on the tracker\.\]$/)
})

test('the commands that land or push work', () => {
  for (const command of ['just land "x" a', 'cd /w && ./tools/land "x" a', '~/workspace/claude/tools/land "x" a', 'jj git push -b x', 'git -C /w push', 'git push origin x', 'git -c push.default=current push', 'git push origin x && head -n 3 log']) {
    expect([command, lands(command)]).toEqual([command, true])
  }
  for (const command of ['just land-preview', 'git status', 'echo just', 'rg "git push" notes.md', 'git push --dry-run origin x', 'jj git push -n -b x']) {
    expect([command, lands(command)]).toEqual([command, false])
  }
})

/** The arguments the mod last ran hooks/commits.sh with. */
const commitsRun = (runs: (readonly string[])[]) => runs.filter((argv) => argv[1]?.endsWith('/hooks/commits.sh')).at(-1)?.slice(2)

test("a session's coin picks the arm it runs first, and it runs the other from the switch on", async () => {
  const trial = assignment(B_FIRST)
  expect([await armOf(trial, 'lane-a', 0), await armOf(trial, 'lane-b', 0)]).toEqual(['B', 'A'])
  expect([await armOf(trial, 'lane-a', DAY - 1), await armOf(trial, 'lane-b', DAY - 1)]).toEqual(['B', 'A'])
  expect([await armOf(trial, 'lane-a', DAY), await armOf(trial, 'lane-b', DAY)]).toEqual(['A', 'B'])
})

test('an A/B file without a seed or a time to switch at is refused', () => {
  for (const text of ['{"seed": "s3"}', '{"seed": "", "switchAt": "1970-01-02T00:00:00Z"}', '{"seed": "s3", "switchAt": "tomorrow"}', 'null']) {
    expect(() => assignment(text)).toThrow('it needs a seed and a switchAt time')
  }
})

test('in arm B the first prompt carries no state file, but the open issues and the commits in the workspaces its cwd and transcript name', async ($, on) => {
  const { seen } = world($, on, { ab: B_FIRST })
  await start($)
  await prompt($, 'hello')
  const [items, commits, mail, ...rest] = seen.prompts[0].context ?? []
  expect(items).toContain('### #7 make the thing\nhttps://w/7')
  expect(commits).toContain('### /w (jj)\nWorking copy: zzzzzzzz aaaa1111')
  expect(mail).toContain('- topics: harness')
  expect(rest).toEqual([])
  expect(commitsRun(seen.runs)).toEqual(['/w', transcript('s1')])
})

test("arm B's ask has the lane leave its notes on its issues, filing one for a concern none covers", async ($, on) => {
  world($, on, { ab: B_FIRST })
  await start($)
  const [asked] = (await bash($, 'jj git push --bookmark x')).additionalContext ?? []
  expect(asked).toContain('You landed work this turn and the context holds 250k tokens, so this context is due to start over.')
  expect(asked).toContain('Write what it needs beyond those as a comment on the issue it concerns')
  expect(asked).toContain('The next context is handed your open issues, each with the newest comment by anyone else and your own newest,')
  expect(asked).toContain(
    'A concern no open issue covers gets a new issue labelled owner/lane-a, and priority/backlog if nothing acts on it now, with a "Closes with:" line in its body saying what ends it, and the note as its first comment. ' +
      'Close it only once that holds: the next context is handed open issues alone.',
  )
  expect(asked).toContain('say in a comment on its issue what each is for and what to do with its result')
  expect(asked).toContain('If you are partway through a step that a comment cannot carry, finish the step first.')
  expect(asked).not.toContain('state file')
})

test('in arm B rotate needs no state file, changed or not', async ($, on) => {
  const { clock } = world($, on, { ab: B_FIRST, linked: false })
  await start($)
  await clock.advance(1_000)
  await bash($, 'git push')
  expect(await rotateCall($)).toEqual({ result: 'the context starts over when this turn ends; end the turn now' })
})

test('a rotation in arm B hands the fresh context the issues, the commits, what the cleared one was sent and what ran, then has it carry on', async ($, on) => {
  const { seen, clock, appended } = world($, on, { ab: B_FIRST, linked: false })
  await start($)
  await prompt($, 'hello')
  await rotateCall($)
  await end($)
  await clock.advance(500)
  const [kit] = appended()
  expect(kit).toHaveLength(5)
  expect(kit[0]).toContain('### #7 make the thing')
  expect(kit[1]).toContain('### /w (jj)')
  expect(kit[2]).toContain(`(its watch record, ${MAIL}/s2.json):\n- topics: harness`)
  expect(kit[3]).toContain('dispatch → lane-a: go')
  expect(kit[4]).toBe('Running in the background when the last turn ended (anything started since is not listed):\n- nothing')
  expect(commitsRun(seen.runs)).toEqual(['/w', transcript('s1')])
  expect(seen.prompts.at(-1)).toEqual({ text: CARRY_ON_B })
})

test('in arm B a compaction keeps the issues, the commits, what was sent and what ran around the summary', async ($, on) => {
  world($, on, { ab: B_FIRST })
  compactions(on)
  await start($)
  await stop($, [shell('cargo test')])
  const texts = (await $.session.compact({ trigger: 'auto', messages: [OLD] })).messages?.map(({ text }) => text) ?? []
  expect(texts).toHaveLength(7)
  expect(texts[0]).toContain('### #7 make the thing')
  expect(texts[1]).toContain('### /w (jj)')
  expect(texts[2]).toContain('- topics: harness')
  expect(texts[3]).toContain('dispatch → lane-a: go')
  expect(texts[4]).toBe('Running in the background when the last turn ended (anything started since is not listed):\n- shell: cargo test')
  expect(texts[5]).toBe('the summary')
})

test('from the switch on a session runs its other arm', async ($, on) => {
  const { seen, clock } = world($, on, { ab: B_FIRST })
  await clock.advance(DAY)
  await start($)
  await prompt($, 'hello')
  expect(seen.prompts[0].context?.[0]).toContain(`Your state file is ${STATE}`)
  expect(commitsRun(seen.runs)).toBeUndefined()
})

test('an A/B file that cannot be read is reported, and the session runs arm A', async ($, on) => {
  const { seen } = world($, on, { ab: '{"seed": "s3"}' })
  await start($)
  await prompt($, 'hello')
  expect(seen.prompts[0].context?.[0]).toContain(`Your state file is ${STATE}`)
  expect(seen.logs).toContain(`lane: ${AB} was not read, so this session runs arm A: it needs a seed and a switchAt time`)
})

test('commits that cannot be listed are named in their place, and the rest still loads', async ($, on) => {
  const { seen } = world($, on, { ab: B_FIRST, commits: ran('', 1, 'jj: Error: There is no jj repo in "."') })
  await start($)
  await prompt($, 'hello')
  expect(seen.prompts[0].context).toEqual([
    expect.stringContaining('### #7 make the thing'),
    'The commits in the workspaces this session used could not be listed (jj: Error: There is no jj repo in "."). Look at them yourself (jj log, git log) before you pick up work.',
    expect.stringContaining('- topics: harness'),
  ])
})

test('each workspace shows its working copy, the commits below it that trunk lacks, and the others its output named', () => {
  expect(commitsBlock([JJ, GIT])).toBe(
    'The workspaces this session used: those its transcript names, most recently first (the -R of its jj commands, the -C of its git ones, ' +
      "where it cd'd and where it edited files), then its cwd. Each with its working copy as the last command run in it left it, the commits below " +
      'that trunk lacks, and other commits its commit, push and land output named.\n\n' +
      '### /w (jj)\n' +
      'Working copy: zzzzzzzz aaaa1111 2026-11-09 12:00Z (no description) (empty), not pushed\n' +
      'Below it and not on trunk, newest first:\n' +
      '- yyyyyyyy bbbb2222 2026-11-09 11:00Z mods/lane: arm B [harness/v12-lane-mod], pushed\n' +
      '- [3 more]\n' +
      'Named in its output:\n' +
      '- xxxxxxxx cccc3333 2026-11-09 10:00Z lanes: landed [master], pushed\n\n' +
      '### /g (git)\n' +
      'HEAD: 1234567 2026-11-09 09:00Z g2 [main], not pushed; 2 files changed in the working tree\n' +
      'Below it and not on trunk: none.',
  )
})

test('workspaces past the cap are counted, and none found is said', () => {
  const big = (i: number): Workspace => ({ ...JJ, root: `/w${i}`, line: Array.from({ length: 10 }, () => ({ ...JJ.line[0], subject: 'x'.repeat(300) })) })
  const block = commitsBlock(Array.from({ length: 6 }, (_, i) => big(i)))
  expect(block.length).toBeLessThan(CAPS.commits + 600)
  expect(block).toMatch(/\n\n\[\d more workspaces are left out\.\]$/)
  expect(commitsBlock([])).toBe("No jj or git workspace was found in this session's cwd or transcript.")
})

test('a session with no watch record, or one that cannot be read, is told so in its place', async ($, on) => {
  const { seen, files } = world($, on)
  delete files[`${MAIL}/s1.json`]
  await start($)
  await prompt($, 'hello')
  expect(seen.prompts[0].context?.[4]).toBe(
    `This session watches no agent-mail topic or thread: its watch record, ${MAIL}/s1.json, does not exist. Run agent-mail wait --topic lane-a once, so its mail reaches it.`,
  )
  files[`${MAIL}/s1.json`] = '{"topics": "harness"}'
  await sessionStart($, 'resume')
  await prompt($, 'again')
  expect(seen.prompts[1].context?.[4]).toBe(
    `This session's watch record, ${MAIL}/s1.json, could not be read (its topics and threads must be lists of names). Run agent-mail wait --topic lane-a once, so its mail reaches it.`,
  )
})

test('the threads watched are listed newest first, and those past the cap are counted', () => {
  expect(mailBlock('/m.json', 'lane-a', { topics: ['harness', 'dispatch'], threads: ['h1', 'h2', 'h3'] })).toBe(
    'This session watches these agent-mail topics and threads (its watch record, /m.json):\n- topics: harness, dispatch\n- threads, newest first: h3, h2, h1',
  )
  const many = Array.from({ length: 2_000 }, (_, i) => `h32aa${String(i).padStart(4, '0')}`)
  const block = mailBlock('/m.json', 'lane-a', { topics: [], threads: many })
  expect(block).toContain('- topics: none\n- threads, newest first: h32aa1999, h32aa1998')
  expect(block.length).toBeLessThan(CAPS.mail + 300)
  expect(block).toMatch(/ \[\d+ older\]$/)
})

test('a rotate tool the engine refused before a session was bound is offered at the next start hook, and once', async ($, on) => {
  const { seen } = world($, on, { refusedRegisters: 1 })
  await sessionStart($, 'startup')
  expect(seen.tools).toEqual([])
  await $.session.start({ cwd: '/w', surface: 'terminal', isInteractive: true })
  await prompt($, 'hello')
  expect(seen.tools).toEqual(['rotate'])
})

test('a rotate tool refused at every start hook is offered at the first prompt, before any turn could call it', async ($, on) => {
  const { seen } = world($, on, { refusedRegisters: 2 })
  await sessionStart($, 'startup')
  await $.session.start({ cwd: '/w', surface: 'terminal', isInteractive: true })
  expect(seen.tools).toEqual([])
  await prompt($, 'hello')
  expect(seen.tools).toEqual(['rotate'])
})
