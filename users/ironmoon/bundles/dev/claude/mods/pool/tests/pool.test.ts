import type { On } from 'claude-code'
import { type Engine, expect, mock, test } from 'claude-code/testing'

const SOCK = '/run/user/1000/anthropic-tap/control.sock'
const UNIT = `Environment=STATE_DIR=/home/u/.local/state/anthropic-tap CONTROL_SOCK=${SOCK} LISTEN_PROXY=127.0.0.1:8889`

type Tap = { always: string[]; session: string[]; keeps: boolean }

/**
 * The tap's control socket over its rules and switches, the session the engine
 * is on, and each request it was asked.
 */
function stubTap(on: On, { unit = UNIT, always = [], session = ['pool'], keeps = true }: Partial<Tap> & { unit?: string } = {}) {
  const tap = { switches: new Map<string, string>(), seen: new Set<string>(), asked: [] as string[], session: 's1', clock: mock.clock(on) }
  on('session.id', () => ({ value: tap.session }))
  on('process.run', () => ({ value: { exitCode: 0, stdout: `${unit}\n`, stderr: '', isStdoutTruncated: false, isStderrTruncated: false } }))
  on('command.register', (_$, e) => ({ value: { command: e.name } }))
  on('session.start', (_$, e) => ({ cwd: e.cwd }))
  on('session.end', (_$, e) => ({ sessionId: e.sessionId }))
  on('turn.start', (_$, e) => ({ turnId: e.turnId }))
  on('turn.complete', () => ({ text: '', reason: 'answer' }))
  on('http.fetch', (_$, { url, init }) => {
    expect(init?.socketPath).toBe(SOCK)
    const id = new URL(url).pathname.replace('/sessions/', '')
    const said = (status: number, text: string) => ({ value: { status, ok: status === 200, headers: {}, text } })
    tap.asked.push(init?.method === 'PUT' ? `${id} ${init.body}` : id)
    if (init?.method === 'PUT') {
      if (session.length === 0) return said(409, 'this tap runs no session rule to switch')
      if (!keeps) return said(500, 'the switch could not be kept')
      tap.switches.set(id, init.body ?? '')
    }
    const switched = tap.switches.get(id) ?? 'off'
    const through = always.includes('pool') || (switched === 'on' && session.includes('pool')) ? 'pool' : null
    return said(200, JSON.stringify({ switch: switched, through, seen: tap.seen.has(id) }))
  })
  return tap
}

const START = { cwd: '/home/u/proj', surface: 'terminal', isInteractive: true } as const
const command = (command: string, args = '') =>
  ({ command, args, origin: { kind: 'composer' }, presentation: { isFullscreen: false, columns: 120 } }) as const
const pool = (args = '') => command('pool', args)
const turn = (agentId?: string) => ({ text: 'hi', turnId: 't', ...(agentId && { agentId }) })
const done = (agentId?: string) =>
  ({ answer: '', durationMs: 1, isAborted: false, turnId: 't', reason: 'answer', ...(agentId && { agentId }) }) as const

/** Another mod, reading the route the pool mod keeps for it. */
const READER = {
  name: 'reader',
  register(on: On) {
    on('session.start', async ($, e, next) => {
      const started = await next(e)
      await $.command.register({ name: 'peek', description: 'pool.route' })
      return started
    })
    on('command.run', { command: 'peek' }, async ($) => ({
      text: JSON.stringify((await $.state.get({ plugin: 'pool', key: 'route' })).value),
    }))
  },
}
const peek = async ($: Engine) => JSON.parse((await $.command.run(command('peek'))).text ?? 'null')

test('/pool flips this session on and off', { plugins: [READER] }, async ($, on) => {
  const tap = stubTap(on)
  await $.session.start(START)
  expect((await $.command.run(pool())).text).toBe(
    "on: this session's Claude turns go through pool; the tap has carried none of them yet",
  )
  expect(await peek($)).toEqual({ switch: 'on', through: 'pool', seen: false })
  tap.seen.add('s1')
  expect((await $.command.run(pool())).text).toBe("off: this session's Claude turns go to your own account")
  expect(tap.asked.filter((asked) => asked.includes(' '))).toEqual(['s1 on', 's1 off'])
})

test('/pool on and off set, status only reads, and anything else is refused', async ($, on) => {
  const tap = stubTap(on)
  await $.session.start(START)
  await $.command.run(pool('on'))
  expect((await $.command.run(pool('status'))).text).toMatch(/^on: /)
  expect((await $.command.run(pool('maybe'))).text).toBe('usage: /pool [on | off | status]')
  expect(tap.asked.filter((asked) => asked.includes(' '))).toEqual(['s1 on'])
})

test('the row learns the tap carried the session once a main turn completes', { plugins: [READER] }, async ($, on) => {
  const tap = stubTap(on)
  await $.session.start(START)
  await $.command.run(pool('on'))
  await $.turn.start(turn())
  tap.seen.add('s1')
  expect(await peek($)).toMatchObject({ seen: false })
  await $.turn.complete(done())
  expect(await peek($)).toEqual({ switch: 'on', through: 'pool', seen: true })
})

test('a subagent turn asks the tap nothing', async ($, on) => {
  const tap = stubTap(on)
  await $.session.start(START)
  const asked = tap.asked.length
  await $.turn.start(turn('a1'))
  await $.turn.complete(done('a1'))
  expect(tap.asked.length).toBe(asked)
})

test('after a /clear the row shows the new session at the tap’s default', { plugins: [READER] }, async ($, on) => {
  const tap = stubTap(on)
  await $.session.start(START)
  await $.command.run(pool('on'))
  await $.session.end({ reason: 'clear', sessionId: 's1', resume: { id: 's1' } })
  tap.session = 's2'
  await tap.clock.advance(5_000)
  expect(await peek($)).toEqual({ switch: 'off', through: null, seen: false })
})

test('the row follows a tap that restarted', { plugins: [READER] }, async ($, on) => {
  const tap = stubTap(on)
  await $.session.start(START)
  await $.command.run(pool('on'))
  tap.seen.add('s1')
  await tap.clock.advance(5_000)
  expect(await peek($)).toMatchObject({ seen: true })
  tap.seen.clear()
  await tap.clock.advance(5_000)
  expect(await peek($)).toMatchObject({ seen: false })
})

test('a session nobody sits at asks the tap only around its turns', async ($, on) => {
  const tap = stubTap(on)
  await $.session.start({ ...START, surface: null, isInteractive: false })
  const asked = tap.asked.length
  await tap.clock.advance(60_000)
  expect(tap.asked.length).toBe(asked)
})

test('a switch the tap cannot keep is reported unchanged', async ($, on) => {
  stubTap(on, { keeps: false })
  await $.session.start(START)
  expect((await $.command.run(pool('on'))).text).toBe('the switch is unchanged: the switch could not be kept')
})

test('a tap whose pool takes every session says so, and has nothing to switch', async ($, on) => {
  stubTap(on, { always: ['pool'], session: [] })
  await $.session.start(START)
  expect((await $.command.run(pool('status'))).text).toMatch(/^off: this session's Claude turns go through pool/)
  expect((await $.command.run(pool('on'))).text).toBe('the switch is unchanged: this tap runs no session rule to switch')
})

test('a tap unit with no control socket leaves /pool saying so', async ($, on) => {
  const tap = stubTap(on, { unit: 'Environment=STATE_DIR=/x LISTEN_PROXY=127.0.0.1:8889' })
  await $.session.start(START)
  expect((await $.command.run(pool('on'))).text).toBe(
    'no tap answers for this session: anthropic-tap.service sets no CONTROL_SOCK',
  )
  expect(tap.asked).toEqual([])
})
