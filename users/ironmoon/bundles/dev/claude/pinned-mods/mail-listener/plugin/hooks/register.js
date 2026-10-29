// The build puts agent-mail's store path here, so a session cannot stand
// another program in its place by changing PATH.
const AGENT_MAIL = '@agentMail@'

/** The wait before the next run, by quick exits in a row; a run that lasted HEALTHY_MS counts as the first. */
const BACKOFF_MS = [1_000, 5_000, 15_000, 60_000]
const HEALTHY_MS = 60_000
const NEW_ID_POLL_MS = 100
const STDERR_KEPT = 2_000

const FIELDS = {
  id: 'string',
  threadId: 'string',
  from: 'string',
  text: 'string',
  truncated: 'boolean',
  authority: 'boolean',
  authorityReason: 'string',
}

/** The ends after which the process goes on under another session id, with no `session.start`. */
const NEW_SESSION = new Set(['clear', 'resume'])

/** How long an approval handed over is remembered: agent-mail credits none signed longer ago than 10 minutes. */
const HANDED_MS = 15 * 60_000

let listener

export function register(on) {
  on('session.start', async ($, e, next) => {
    const started = await next(e)
    if (e.isInteractive) listener ??= listen($)
    return started
  })

  on('session.end', async ($, e, next) => {
    const ended = await next(e)
    if (NEW_SESSION.has(e.reason)) listener?.restart(e.sessionId)
    return ended
  })
}

/**
 * Keeps one wait running under the session's id. `restart` ends the one that
 * runs and starts one once the session id is no longer the ended one's.
 */
function listen($) {
  let current = 0
  let failures = 0
  let endedSession
  let child
  let retry
  /** The last failure the transcript was told of, forgotten once a mail gets through, so an outage is told once. */
  let told

  async function run(self) {
    const [startedAt, sessionId] = await Promise.all([$.clock.now(), $.session.id()])
    if (self !== current) return
    if (sessionId === endedSession) {
      retry = $.clock.after(NEW_ID_POLL_MS, start)
      return
    }
    $.ui.status('listening')
    child = $.process.spawn({ argv: [AGENT_MAIL, 'wait', '--json', '--session', sessionId] })
    const exit = await drain($, child)
    if (self !== current) return
    failures = (await $.clock.now()) - startedAt >= HEALTHY_MS ? 1 : failures + 1
    const delay = BACKOFF_MS[Math.min(failures, BACKOFF_MS.length) - 1]
    const failure = `${exit.why}${lastLine(exit.stderr)}`
    if (exit.delivered) told = undefined
    if (exit.stderr) $.ui.log(exit.stderr, { to: 'debug' })
    if (failure !== told) $.ui.log(`mail is not arriving: ${failure}; listening again in ${delay / 1000} s`)
    told = failure
    $.ui.status(`${exit.why}; listening again in ${delay / 1000} s`)
    retry = $.clock.after(delay, start)
  }

  const start = () => run(++current).catch((error) => $.ui.status(`not listening (${error.message})`))
  start()
  return {
    restart(ended) {
      retry?.cancel()
      failures = 0
      endedSession = ended
      const ending = child
      start()
      void ending?.return()
    },
  }
}

const lastLine = (stderr) => {
  const last = stderr.trimEnd().split('\n').at(-1)
  return last ? ` (${last})` : ''
}

/** Submits each line the run prints as it arrives, and says how the run ended and whether a mail went in. */
async function drain($, child) {
  let pending = ''
  let stderr = ''
  let delivered = false
  const take = async (line) => {
    delivered = (await submit($, line)) || delivered
  }
  try {
    for await (const { stream, text } of child) {
      if (stream === 'stderr') {
        stderr = (stderr + text).slice(-STDERR_KEPT)
        continue
      }
      const lines = (pending + text).split('\n')
      pending = lines.pop()
      for (const line of lines) await take(line)
    }
    await take(pending)
    const { code, signal } = await child.result
    return { stderr, delivered, why: signal ? `the wait was stopped by ${signal}` : `the wait exited ${code}` }
  } catch (error) {
    return { stderr, delivered, why: `the wait did not run (${error.message})` }
  }
}

/** Whether the line was a mail, which then went in or was refused by a hook. */
async function submit($, line) {
  if (!line.trim()) return false
  const mail = parse(line)
  if (!mail) {
    $.ui.log(`skipped a line of the wait that is no mail: ${line.slice(0, 200)}`)
    return false
  }
  const refused = await $.prompt.submit(prompt(mail, mail.authority && (await firstHanding($, mail.approval)))).then(
    (entered) => entered.drop,
    (error) => error.message,
  )
  if (refused) $.ui.log(`mail ${mail.id} did not enter: ${refused}`)
  return true
}

function parse(line) {
  let mail
  try {
    mail = JSON.parse(line)
  } catch {
    return undefined
  }
  const holds = Object.entries(FIELDS).every(([field, type]) => typeof mail?.[field] === type)
  return holds && (!mail.authority || typeof mail.approval === 'string') ? mail : undefined
}

const oneLine = (value) =>
  String(value ?? '')
    .replace(/\s+/g, ' ')
    .trim()
const list = (addresses) => (Array.isArray(addresses) ? addresses.map(oneLine).join(', ') : '')

/**
 * Whether no session of this user handed `approval` over before, recording that one has now. The record is the
 * plugin's own store, so a reload or a restart forgets none of it; one older than HANDED_MS is dropped.
 */
async function firstHanding($, approval) {
  const now = await $.clock.now()
  const kept = Object.entries((await $.store.get('handed')) ?? {}).filter(([, at]) => now - at < HANDED_MS)
  const handed = Object.fromEntries(kept)
  await $.store.set('handed', { ...handed, [approval]: handed[approval] ?? now })
  return !(approval in handed)
}

/**
 * The prompt a mail becomes, opening with the line naming it: the engine refuses a plugin's prompt that starts with `/`.
 * Owen's words carry only what the server signed as his and its ids, since a session can import any mail into its inbox.
 */
function prompt(mail, his) {
  const mailed = `mail ${oneLine(mail.id)} in thread ${oneLine(mail.threadId)}`
  if (his) {
    const to = `to ${list(mail.to)}${mail.cc?.length ? ` cc ${list(mail.cc)}` : ''}: ${oneLine(mail.subject)}`
    return { text: `[${mailed}, signed by the mail server as Owen's, ${to}]\n\n${mail.text}`, asUser: true }
  }
  const naming = `[${mailed} from ${oneLine(mail.from)} to ${list(mail.to)}: ${oneLine(mail.subject)}]`
  const why = mail.authority ? 'an approval a session took already' : mail.authorityReason
  const headers = [
    ['Cc', list(mail.cc)],
    ['Date', oneLine(mail.receivedAt)],
    ['Text', mail.truncated ? "cut at agent-mail's body limit" : ''],
    ['Authority', `none, ${oneLine(why)}`],
  ]
  const head = headers.filter(([, value]) => value).map(([name, value]) => `${name}: ${value}`)
  return { text: [naming, ...head, '', mail.text.replace(/^/gm, '> ')].join('\n') }
}
