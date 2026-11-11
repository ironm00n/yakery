import type { EngineInterface, HookFailure, Register, SessionMessage } from 'claude-code'
import * as kit from './kit'

const TOOL = 'rotate'
const STARTS_OVER = 'the context starts over when this turn ends; end the turn now'
/** Context tokens past which landed work starts the context over, and past which any turn's end does. */
const UNIT = 200_000
const MARK = 600_000
/** An ask left unanswered is put again once the context has grown this much past it. */
const AGAIN = 100_000
/** `/clear` waits for the turn that asked for it to be over, which the engine may still be ending. */
const WAIT_MS = 500
/** How long each part of the kit may take, since the prompt that brings it waits for it. */
const KIT_MS = 30_000
const TRIES = 20
/** The built-in tools sessions loaded through ToolSearch more than 50 times each in the 14 days to 11-09 (285 transcripts),
 * listed in front for a named session so it never has to ask for them. */
const UPFRONT = new Set(['SendMessage', 'TaskStop', 'CronList', 'Monitor', 'WebSearch', 'CronCreate', 'CronDelete', 'WebFetch'])

type Lane = { name: string; home: string; link: string; pid: number }
type Session = { id: string; transcript: string }
type Classic = { session_id: string; transcript_path: string; agent_id?: string }

/** The session's name and where its state file is linked from: kept across `/clear` and compaction, looked up again after
 * a start, resume or fork, since the process may now hold another session. */
let lane: Lane | undefined
/** The session id last looked up and found unnamed, and when: the lookup waits a minute before trying it again, rather
 * than running for each tool the engine describes, and a record written late is still found. */
let unnamed: { id: string; at: number } | undefined
const UNNAMED_MS = 60_000
/** The main conversation's session, from the latest classic hook that named it, so a reload of this module recovers it. */
let session: Session | undefined
/** The next prompt of the main conversation carries the kit, and the cleared context's messages when there was one. */
let pending: { after?: Session } | undefined
/** A `/clear` this mod ran, which hands over the kit itself rather than leaving it to the next prompt. */
let isOwnClear = false
/** The rotate tool's registration, once asked for; one the engine refused (it does, before a session is bound to the
 * process) is asked for again at the next start hook or prompt. */
let offered: Promise<unknown> | undefined
let tasks: kit.Task[] = []
let crons: kit.Cron[] = []
let landed = false
/** The context's size at the last ask, by where it came (right after a command that landed work, or at a turn's end), and
 * when the latest came. */
let asked: { landing?: number; end?: number; at?: number } = {}
let rotating = false
/** Per issue the lane filed, how many comments it had when it was last put before the lane in this user turn: it is put
 * again only once another comment is posted to it, and afresh at the next turn someone's words start. */
let shown = new Map<number, number>()
/** Turns no one's new words start: a notification, a schedule, an observer, a follow-up, one the engine can't place, and
 * this mod's own prompt to carry on. */
const MACHINE = new Set(['task-notification', 'scheduled-trigger', 'observer', 'observer-activity', 'auto-continuation', 'unclassified'])
const startsUserTurn = (origin: { kind: string; name?: string } | undefined) =>
  origin !== undefined && !MACHINE.has(origin.kind) && !(origin.kind === 'plugin' && origin.name === 'lane')

const user = (text: string): SessionMessage => ({ role: 'user', text, toolUses: [] })
const why = (failure: unknown) => (failure instanceof Error ? failure.message : String(failure))
const failed = ({ kind, message }: HookFailure) => message ?? (kind === 'timeout' ? 'timed out' : 'it failed')
const lastLine = (text: string) => text.trim().split('\n').at(-1) ?? ''

function note(e: Classic) {
  if (e.agent_id === undefined && session?.id !== e.session_id) session = { id: e.session_id, transcript: e.transcript_path }
}

/** Named sessions only: ~/.claude/sessions/<pid>.json holds each one's name beside its id, and the engine's process. */
async function laneOf($: EngineInterface): Promise<Lane | undefined> {
  if (lane !== undefined) return lane
  const [id, now] = [session?.id ?? (await $.session.id()), await $.clock.now()]
  if (unnamed?.id === id && now - unnamed.at < UNNAMED_MS) return undefined
  const home = (await $.env.get('HOME')) ?? ''
  const dir = `${home}/.claude/sessions`
  const records = await Promise.all(
    (await $.fs.list(dir))
      .filter(({ name }) => name.endsWith('.json'))
      .map(({ name }) => $.fs.read(`${dir}/${name}`).then(JSON.parse).catch(() => undefined)),
  )
  const { name, pid } = records.find((record) => record?.sessionId === id) ?? {}
  if (typeof name === 'string' && name !== '') lane = { name, home, link: `${home}/.local/state/claude-lanes/${name}`, pid }
  else unnamed = { id, at: now }
  return lane
}

const stateFile = async ($: EngineInterface, { link }: Lane) => (await $.fs.stat(link, { resolve: true }).catch(() => undefined))?.realPath

/** The arm the session runs now: A, unless `just lane-ab` has started the trial. A trial file that can't be read is
 * reported, and the session runs A. */
async function armNow($: EngineInterface, { name, home }: Lane): Promise<kit.Arm> {
  const path = `${home}/.local/state/claude-lanes/ab.json`
  if (!(await $.fs.exists(path))) return 'A'
  try {
    return await kit.armOf(kit.assignment(await $.fs.read(path)), name, await $.clock.now())
  } catch (failure) {
    $.ui.log(`lane: ${path} was not read, so this session runs arm A: ${why(failure)}`)
    return 'A'
  }
}

async function askOf($: EngineInterface, me: Lane): Promise<kit.Ask> {
  return (await armNow($, me)) === 'B' ? { arm: 'B', name: me.name } : { arm: 'A', hasState: (await stateFile($, me)) !== undefined }
}

async function stateBlocks($: EngineInterface, me: Lane): Promise<string[]> {
  const path = await stateFile($, me)
  if (path === undefined) return [kit.noStateBlock(me.name, me.link)]
  const text = await $.fs.read(path).catch((failure) => failure as Error)
  if (text instanceof Error) return [kit.unreadStateBlock(path, why(text))]
  const files = await Promise.all(
    kit.carried(text).map(async (file) => {
      const at = file.replace(/^~(?=\/)/, me.home)
      return $.fs.read(at).then(
        (body) => ({ path: at, text: body }),
        (failure) => ({ path: at, text: null, why: why(failure) }),
      )
    }),
  )
  return [kit.stateBlock(me.link, path, text), ...kit.carriedBlocks(files)]
}

/** The lane's open issues; given what the lane posted (arm B), each with the newest of anyone else's comments and the
 * lane's own newest, so a note it left never hides another session's latest word, and those it filed marked to close. */
async function itemsBlock($: EngineInterface, { name }: Lane, own?: Posted): Promise<string> {
  try {
    const mine = own === undefined ? [] : [JSON.stringify(own.comments)]
    const run = await $.process.run(['bash', `${$.plugin.root}/hooks/items.sh`, name, kit.TRACKER, ...mine], { timeoutMs: KIT_MS })
    if (run.exitCode !== 0) return kit.unreadItemsBlock(name, lastLine(run.stderr) || `items.sh exited ${run.exitCode}`)
    return kit.itemsBlock(name, JSON.parse(run.stdout), own?.issues)
  } catch (failure) {
    return kit.unreadItemsBlock(name, why(failure))
  }
}

/** What a context posted to the tracker: the ids of its comments and the numbers of the issues it filed. */
type Posted = { comments: number[]; issues: number[] }
/** One context the lane has had, and once it has ended and been read (arm B), what it posted. */
type Context = Session & Partial<Posted>
const contextsOf = ({ home, name }: Lane) => `${home}/.local/state/claude-lanes/contexts/${name}.jsonl`

/** The lane's contexts, oldest first, from the log the mod keeps, one row each, with `sessions` added if missing: a kit
 * is built at the start of every context, so the log holds each the lane has had since the mod first ran for it. */
async function contextsWith($: EngineInterface, me: Lane, sessions: Session[]): Promise<Context[]> {
  const path = contextsOf(me)
  const rows: Context[] = (await $.fs.exists(path))
    ? (await $.fs.read(path)).split('\n').flatMap((line) => (line === '' ? [] : [JSON.parse(line)]))
    : []
  const added = [...new Map(sessions.map(({ id, transcript }) => [id, { id, transcript }])).values()].filter(({ id }) => !rows.some((row) => row.id === id))
  return [...rows, ...added]
}

const writeContexts = ($: EngineInterface, me: Lane, rows: Context[]) => $.fs.write(contextsOf(me), rows.map((row) => `${JSON.stringify(row)}\n`).join(''))

/** What a context posted (hooks/mine.sh on its transcript); undefined when it can't be read, so it is tried again at the
 * next kit. */
async function postedBy($: EngineInterface, me: Lane, context: Session): Promise<Posted | undefined> {
  try {
    const run = await $.process.run(['bash', `${$.plugin.root}/hooks/mine.sh`, await transcriptOf($, me, context), kit.TRACKER], { timeoutMs: KIT_MS })
    if (run.exitCode === 0) return JSON.parse(run.stdout)
    $.ui.log(`lane: the comments of context ${context.id} were not read: mine.sh exited ${run.exitCode}: ${lastLine(run.stderr)}`)
  } catch (failure) {
    $.ui.log(`lane: the comments of context ${context.id} were not read: ${why(failure)}`)
  }
  return undefined
}

/** Records the current context, and the one this kit follows, in the lane's log; in arm B also reads what each ended
 * context posted, once, and the current one's afresh when a compaction keeps it going, and returns it all. */
async function ownPosts($: EngineInterface, me: Lane, arm: kit.Arm, current: Session, before: Session | undefined): Promise<Posted | undefined> {
  const rows = await contextsWith($, me, [...(before === undefined ? [] : [before]), current])
  const read = arm === 'A' ? rows : await Promise.all(rows.map(async (row) => (row.comments !== undefined || row.id === current.id ? row : { ...row, ...(await postedBy($, me, row)) })))
  await writeContexts($, me, read)
  if (arm === 'A') return undefined
  const going = before?.id === current.id ? await postedBy($, me, current) : undefined
  const all = [...read, ...(going === undefined ? [] : [going])]
  return { comments: all.flatMap(({ comments }) => comments ?? []), issues: all.flatMap(({ issues }) => issues ?? []) }
}

/** The issues the lane filed (arm B, from its log of contexts) that are still open and not yet put before it in this
 * user turn, or put since with no comment on them since; none when there are none, or they can't be read. */
async function stillOpen($: EngineInterface, me: Lane): Promise<kit.Open[] | undefined> {
  try {
    if ((await armNow($, me)) !== 'B') return undefined
    const filed = [...new Set((await contextsWith($, me, [])).flatMap(({ issues }) => issues ?? []))]
    if (filed.length === 0) return undefined
    const run = await $.process.run(['bash', `${$.plugin.root}/hooks/open.sh`, kit.TRACKER, ...filed.map(String)], { timeoutMs: KIT_MS })
    if (run.exitCode !== 0) throw new Error(lastLine(run.stderr) || `open.sh exited ${run.exitCode}`)
    const due = (JSON.parse(run.stdout) as kit.Open[]).filter(({ number, comments }) => (shown.get(number) ?? -1) < comments)
    for (const { number, comments } of due) shown.set(number, comments)
    return due.length === 0 ? undefined : due
  } catch (failure) {
    $.ui.log(`lane: the open issues this lane filed were not checked at the turn's end: ${why(failure)}`)
    return undefined
  }
}

/** Where a session's transcript is: the path its hooks gave, or, once that file is gone (a background job that moved into
 * a worktree writes under that worktree's project directory), the one of its id under any project directory. */
async function transcriptOf($: EngineInterface, { home }: Lane, { id, transcript }: Session): Promise<string> {
  if (await $.fs.exists(transcript)) return transcript
  const projects = `${home}/.claude/projects`
  const dirs = (await $.fs.list(projects).catch(() => [])).filter(({ kind }) => kind === 'dir')
  const found = await Promise.all(dirs.map(({ name }) => `${projects}/${name}/${id}.jsonl`).map(async (path) => ((await $.fs.exists(path)) ? path : undefined)))
  return found.find((path) => path !== undefined) ?? transcript
}

async function saidBlock($: EngineInterface, me: Lane, before: Session): Promise<string> {
  const transcript = await transcriptOf($, me, before)
  try {
    const { owen, peers, message } = kit.CAPS
    const caps = JSON.stringify({ owen, peers, message })
    const jq = await $.process.run(['jq', '-cnR', '--argjson', 'caps', caps, '-f', `${$.plugin.root}/hooks/messages.jq`, transcript], { timeoutMs: KIT_MS })
    if (jq.exitCode !== 0) return kit.unreadSaidBlock(transcript, `messages.jq exited ${jq.exitCode}: ${lastLine(jq.stderr)}`)
    if (jq.isStdoutTruncated) return kit.unreadSaidBlock(transcript, 'they are over the 4 MiB one read returns')
    return kit.saidBlock(JSON.parse(jq.stdout), transcript)
  } catch (failure) {
    return kit.unreadSaidBlock(transcript, why(failure))
  }
}

const watchRecord = ({ home }: Lane, id: string) => `${home}/.local/state/agent-mail/sessions/${id}.json`

/** The mail the session watches now: after a clear, the copy of the cleared one's record that carryMail made. */
async function mailBlock($: EngineInterface, me: Lane): Promise<string> {
  const path = watchRecord(me, await $.session.id())
  try {
    return kit.mailBlock(path, me.name, (await $.fs.exists(path)) ? kit.watch(await $.fs.read(path)) : undefined)
  } catch (failure) {
    return kit.unreadMailBlock(path, me.name, why(failure))
  }
}

/** The workspaces the session used, from its cwd and the transcript of `from`, the context that came before or this one. */
async function commitsBlocks($: EngineInterface, me: Lane, from: Session | undefined): Promise<string[]> {
  try {
    const transcript = from === undefined ? [] : [await transcriptOf($, me, from)]
    const run = await $.process.run(['bash', `${$.plugin.root}/hooks/commits.sh`, await $.session.cwd(), ...transcript], { timeoutMs: KIT_MS })
    if (run.exitCode !== 0) return [kit.unreadCommitsBlock(lastLine(run.stderr) || `commits.sh exited ${run.exitCode}`)]
    return [kit.commitsBlock(JSON.parse(run.stdout))]
  } catch (failure) {
    return [kit.unreadCommitsBlock(why(failure))]
  }
}

/** What a context starting over is handed, by the arm the session runs, what it starts from first: in A the state file
 * and what it carries, then the lane's open issues; in B the open issues, where its notes are, then the commits in its
 * workspaces. Then the mail it watches, and, when it follows `before`, a context of this process (a rotation, a clear, a
 * compaction), the messages sent to that one and what it left running (a clear keeps it) and the crons it set. Each part
 * that fails says so in its place. */
async function kitBlocks($: EngineInterface, me: Lane, before?: Session): Promise<{ arm: kit.Arm; blocks: string[] }> {
  const arm = await armNow($, me)
  const current = session ?? { id: await $.session.id(), transcript: '' }
  const [own, items, mail, said] = await Promise.all([
    arm === 'A' ? stateBlocks($, me) : commitsBlocks($, me, before ?? session),
    ownPosts($, me, arm, current, before)
      .catch((failure) => void $.ui.log(`lane: ${contextsOf(me)} was not read, so this kit tells no comment as the lane's own: ${why(failure)}`))
      .then((comments) => itemsBlock($, me, comments)),
    mailBlock($, me),
    before && saidBlock($, me, before),
  ])
  const after = [mail, ...(said === undefined ? [] : [said, kit.liveBlock(tasks, crons)])]
  return { arm, blocks: arm === 'A' ? [...own, items, ...after] : [items, ...own, ...after] }
}

/** Asked with the result of the command that landed work, so a turn that goes on to its next unit starts that in a fresh
 * context; what still runs is known only at the turn's end, so this ask can't hold back for it. */
async function askAtLanding($: EngineInterface): Promise<string | undefined> {
  const me = await laneOf($)
  if (me === undefined || rotating) return undefined
  const { tokens, window } = (await $.session.usage()).context
  if (tokens === undefined || tokens < Math.min(UNIT, 0.2 * window) || (asked.landing !== undefined && tokens < asked.landing + AGAIN)) return undefined
  asked.landing = tokens
  asked.at = await $.clock.now()
  return kit.askBlock(tokens, 'unit', undefined, await askOf($, me))
}

/** A clear gives the session a new id, which agent-mail keys the topics and threads it watches by. */
async function carryMail($: EngineInterface, me: Lane, from: string, to: string) {
  const [old, now] = [watchRecord(me, from), watchRecord(me, to)]
  try {
    const [had, has] = await Promise.all([$.fs.exists(old), $.fs.exists(now)])
    if (had && !has) await $.fs.write(now, await $.fs.read(old))
  } catch (failure) {
    $.ui.log(`lane: mail subscriptions not carried from ${from} to ${to}: ${why(failure)}`)
  }
}

async function clear($: EngineInterface): Promise<string | undefined> {
  for (let i = 1; ; i++) {
    await $.clock.sleep(WAIT_MS)
    try {
      await $.command.run({ command: 'clear' })
      return undefined
    } catch (failure) {
      if (i === TRIES) return `/clear was refused ${TRIES} times: ${why(failure)}`
    }
  }
}

/** The binary the engine runs, which only its process names: the profile's `claude` can be another version. */
const binaryOf = async ($: EngineInterface, { pid }: Lane) => {
  const exe = await $.process.run(['readlink', '-f', `/proc/${pid}/exe`])
  return exe.exitCode === 0 ? exe.stdout.trim() : undefined
}

/** Each rotation's recall check, hooks/check.sh, runs apart from the session, so it holds up nothing and outlives this
 * process: it asks what the cleared context's last stretch says a successor needs of the kit just handed over, with the
 * engine's own binary, and adds a row to checks/<name>.jsonl. What it reads is left in checks/<name>/<the fresh
 * context's id>/, where it keeps the rest. */
async function check($: EngineInterface, me: Lane, { arm, blocks }: { arm: kit.Arm; blocks: string[] }, before: Session) {
  const checks = `${me.home}/.local/state/claude-lanes/checks`
  const [to, at] = [await $.session.id(), await $.clock.now()]
  const dir = `${checks}/${me.name}/${to}`
  try {
    await $.fs.write(`${dir}/kit.txt`, blocks.join('\n\n'))
    const rotation = {
      lane: me.name,
      arm,
      from: before.id,
      to,
      transcript: await transcriptOf($, me, before),
      notes: [me.link, await stateFile($, me)].filter((path) => path !== undefined),
      tracker: kit.TRACKER,
      at: new Date(at).toISOString(),
      claude: await binaryOf($, me),
      results: `${checks}/${me.name}.jsonl`,
    }
    await $.fs.write(`${dir}/rotation.json`, JSON.stringify(rotation))
    const started = await $.process.run(['setsid', '-f', 'bash', `${$.plugin.root}/hooks/check.sh`, dir])
    if (started.exitCode !== 0) throw new Error(lastLine(started.stderr) || `setsid exited ${started.exitCode}`)
  } catch (failure) {
    $.ui.log(`lane: the recall check of this rotation did not start: ${why(failure)}`)
  }
}

/** The engine may run the clear's SessionStart hooks only before the next prompt, so the kit is built here from the
 * session that was cleared; a plugin's own prompt takes no context, so the kit goes in as rows before it. */
async function rotate($: EngineInterface, me: Lane, before: Session) {
  isOwnClear = true
  const refused = await clear($)
  const now = refused === undefined ? await $.session.id() : before.id
  if (now === before.id) {
    isOwnClear = false
    return $.prompt.submit({ text: kit.notRotated(refused ?? '/clear ran, but the session kept its id') })
  }
  await carryMail($, me, before.id, now)
  try {
    const handed = await kitBlocks($, me, before)
    await $.session.append({ message: { type: 'user', content: handed.blocks.map((text) => ({ type: 'text', text })) } })
    await $.prompt.submit({ text: kit.carryOn(handed.arm) })
    await check($, me, handed, before)
  } catch (failure) {
    await $.prompt.submit({ text: kit.carryOnWithout(await armNow($, me), me.name, me.link, why(failure)) })
  }
}

async function offerRotate($: EngineInterface) {
  if (offered !== undefined || (await laneOf($)) === undefined) return
  offered = $.tool.register({
    name: TOOL,
    description:
      "Start this session's context over when the current turn ends: it is cleared, and the next one begins from what the " +
      'ask to start over names. Call it when asked to, once you have written down what that ask says to, and end the turn right after.',
    inputSchema: { type: 'object', properties: {} },
    isDeferred: false,
  }).catch((failure) => {
    offered = undefined
    $.ui.log(`lane: the rotate tool is not offered yet: ${why(failure)}`, { to: 'debug' })
  })
  await offered
}

export const register: Register = (on) => {
  on('session.start', async ($, e, next) => {
    const started = await next(e)
    await offerRotate($)
    return started
  })

  on('tool.describe', async ($, e, next) => {
    const described = await next(e)
    return UPFRONT.has(e.tool) && (await laneOf($)) !== undefined ? { ...described, isDeferred: false } : described
  })

  on('classic.SessionStart', async ($, e, next) => {
    const done = await next(e)
    const before = session
    session = { id: e.session_id, transcript: e.transcript_path }
    asked = {}
    if (e.source === 'clear') {
      const me = await laneOf($)
      if (isOwnClear) isOwnClear = false
      else if (me !== undefined) {
        pending = { after: before }
        if (before !== undefined) await carryMail($, me, before.id, session.id)
      }
    } else if (e.source !== 'compact') {
      lane = undefined
      unnamed = undefined
      pending = {}
      await offerRotate($)
    }
    return done
  })

  on('prompt.submit', async ($, e, next) => {
    if (startsUserTurn(e.origin)) shown = new Map()
    await offerRotate($)
    const carry = pending
    const me = carry && e.command === undefined ? await laneOf($) : undefined
    if (carry === undefined || me === undefined) {
      if (e.command === undefined) pending = undefined
      return next(e)
    }
    const { blocks } = await kitBlocks($, me, carry.after)
    const entered = await next({ ...e, context: [...(e.context ?? []), ...blocks] })
    if (entered.drop === undefined && entered.context?.includes(blocks[0])) pending = undefined
    return entered
  }).catch(($, e, next) => {
    $.ui.log(`lane: the kit was not loaded: ${failed(next.error)}`)
    return next(e)
  })

  on('classic.PostToolUse', async ($, e, next) => {
    note(e)
    const done = await next(e)
    const command = (e.tool_input as { command?: unknown } | undefined)?.command
    if (e.agent_id !== undefined || e.tool_name !== 'Bash' || typeof command !== 'string' || !kit.lands(command)) return done
    landed = true
    const ask = await askAtLanding($)
    return ask === undefined ? done : { ...done, additionalContext: [...(done.additionalContext ?? []), ask] }
  })

  on('classic.PreCompact', async ($, e, next) => {
    note(e)
    return next(e)
  })

  on('classic.Stop', async ($, e, next) => {
    const done = await next(e)
    if (e.agent_id !== undefined) return done
    note(e)
    tasks = e.background_tasks ?? []
    crons = e.session_crons ?? []
    const wasLanded = landed
    landed = false
    const me = await laneOf($)
    if (me === undefined || e.stop_hook_active || rotating || done.block !== undefined) return done
    const { tokens, window } = (await $.session.usage()).context
    if (tokens === undefined) return done
    const busy = kit.inFlight(tasks)
    const due =
      tokens >= Math.min(MARK, 0.6 * window) ? 'mark' : wasLanded && busy.length === 0 && tokens >= Math.min(UNIT, 0.2 * window) ? 'unit' : undefined
    if (due !== undefined && (asked.end === undefined || tokens >= asked.end + AGAIN)) {
      asked.end = tokens
      asked.at = await $.clock.now()
      return { ...done, block: kit.askBlock(tokens, due, busy, await askOf($, me)) }
    }
    const open = await stillOpen($, me)
    return open === undefined ? done : { ...done, block: kit.closeBlock(open) }
  })

  on('tool.call', { tool: `mcp__lane__${TOOL}` }, async ($, e) => {
    if (e.agentId !== undefined) return { deny: 'only the main conversation starts its context over' }
    const me = await laneOf($)
    if (me === undefined) return { deny: 'this session has no name, so no kit a new context could start from' }
    if ((await armNow($, me)) === 'B') {
      rotating = true
      return { result: STARTS_OVER }
    }
    const path = await stateFile($, me)
    if (path === undefined) return { deny: `${me.link} leads nowhere: make the state file first (\`just lane-state ${me.name}\`)` }
    if (asked.at !== undefined && ((await $.fs.stat(path).catch(() => undefined))?.mtimeMs ?? 0) < asked.at)
      return { deny: `${path} has not changed since you were asked to start over, and it is what the next context starts from: update it first. If writing it is refused, say so and end your turn without rotating.` }
    rotating = true
    return { result: STARTS_OVER }
  })

  on('turn.complete', async ($, e, next) => {
    const done = await next(e)
    if (e.agentId === undefined) {
      const go = rotating
      rotating = false
      const [me, before] = [lane, session]
      if (go && e.reason === 'answer' && me !== undefined && before !== undefined) {
        void rotate($, me, before).catch((failure) => $.ui.log(`lane: the rotation failed: ${why(failure)}`))
      }
    }
    return done
  })

  on('session.compact', async ($, e, next) => {
    if (e.agentId !== undefined || e.trigger === 'precompute') return next(e)
    const compacted = await next(e)
    const me = await laneOf($)
    if (compacted.skip !== undefined || me === undefined || session === undefined) return compacted
    const { blocks } = await kitBlocks($, me, session)
    const { tokensAfter: _stale, ...counts } = compacted
    const pointer = kit.pointerBlock(await transcriptOf($, me, session))
    return { ...counts, messages: [...blocks.map(user), ...compacted.messages, user(pointer)] }
  }).catch(async ($, e, next) => {
    const stock = await next(e)
    $.ui.log(`lane: nothing was carried across this compaction: ${failed(next.error)}`)
    if (stock.skip !== undefined) return stock
    const { tokensAfter: _stale, ...counts } = stock
    const link = lane !== undefined && (await armNow($, lane)) === 'A' ? lane.link : undefined
    return { ...counts, messages: [...stock.messages, user(kit.notCarried(failed(next.error), link))] }
  })
}
