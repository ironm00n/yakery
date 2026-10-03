import type { EngineInterface, Register } from 'claude-code'
import type { PoolRoute } from '../types'

// the unit's environment names the control socket, as it names the proxy to mkProxyApp
const UNIT = 'anthropic-tap.service'
const SOCK = 'CONTROL_SOCK'

const ROUTE = { plugin: 'pool', key: 'route' } as const

// a /clear, a /resume or a fork changes the session's id after every hook that
// sees it has run, and the tap can restart, so the row is asked for again
const RESYNC_MS = 5_000

const SWITCHES = ['on', 'off'] as const
type Switch = (typeof SWITCHES)[number]
const isSwitch = (word: string): word is Switch => (SWITCHES as readonly string[]).includes(word)

/** The control socket's path; null when the unit names none, undefined until read. */
let sock: string | null | undefined
// a hot reload drops this environment's timer along with the flag
let isTicking = false
/** A tick finds the last one still asking a tap that does not answer. */
let isSyncing = false

async function controlSock($: EngineInterface): Promise<string | null> {
  const unit = await $.process.run(['systemctl', '--user', 'show', UNIT, '-p', 'Environment'])
  const words = unit.exitCode === 0 ? unit.stdout.trim().replace(/^Environment=/, '').split(/\s+/) : []
  return words.find((word) => word.startsWith(`${SOCK}=`))?.slice(SOCK.length + 1) ?? null
}

/** The tap's answer for this session, after asking it to switch the session to `set`. */
async function ask($: EngineInterface, set?: Switch): Promise<PoolRoute> {
  if (sock === null) return { unavailable: `${UNIT} sets no ${SOCK}` }
  const init = set === undefined ? {} : { method: 'PUT', body: set }
  const url = `http://tap/sessions/${await $.session.id()}`
  const answer = await $.http.fetch(url, { ...init, socketPath: sock }).catch(() => undefined)
  if (answer === undefined) return { unavailable: `nothing answers at ${sock}` }
  return answer.ok ? JSON.parse(answer.text) : { unavailable: answer.text }
}

/** Asks the tap where this session's turns go, and keeps the answer for the row. */
async function sync($: EngineInterface, set?: Switch, fresh = false): Promise<PoolRoute> {
  if (fresh || sock === undefined) sock = await controlSock($)
  const route = await ask($, set)
  // the host's own copy, which it empties at every session's end
  const { value: shown } = await $.state.get(ROUTE)
  if (JSON.stringify(shown) !== JSON.stringify(route)) await $.state.set(ROUTE, route)
  return route
}

function said(route: PoolRoute): string {
  if ('unavailable' in route) return `no tap answers for this session: ${route.unavailable}`
  const where = route.through === null ? 'to your own account' : `through ${route.through}`
  const carried = route.seen ? '' : '; the tap has carried none of them yet'
  return `${route.switch}: this session's Claude turns go ${where}${carried}`
}

export const register: Register = (on) => {
  on('session.start', async ($, e, next) => {
    const started = await next(e)
    await $.command.register({
      name: 'pool',
      description: "Send this session's turns through the seat pool, or back to your own account",
      argumentHint: 'on | off | status',
    })
    await sync($, undefined, true)
    if (e.isInteractive && !isTicking) {
      isTicking = true
      $.clock.every(RESYNC_MS, async () => {
        if (isSyncing) return
        isSyncing = true
        await sync($).finally(() => (isSyncing = false))
      })
    }
    return started
  })

  // before a turn's requests, and after them, once the tap has carried some
  on('turn.start', async ($, e, next) => {
    if (e.agentId === undefined) await sync($)
    return next(e)
  })
  on('turn.complete', async ($, e, next) => {
    const completed = await next(e)
    if (e.agentId === undefined) await sync($)
    return completed
  })

  on('command.run', { command: 'pool' }, async ($, e) => {
    const word = e.args.trim()
    const now = await sync($, undefined, true)
    if (word === 'status' || 'unavailable' in now) return { text: said(now) }
    const wanted = word === '' ? (now.switch === 'on' ? 'off' : 'on') : word
    if (!isSwitch(wanted)) return { text: 'usage: /pool [on | off | status]' }
    const switched = await sync($, wanted)
    if ('unavailable' in switched) {
      await sync($)
      return { text: `the switch is unchanged: ${switched.unavailable}` }
    }
    return { text: said(switched) }
  })
}
