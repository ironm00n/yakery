import type { On } from 'claude-code'
import { type Engine, expect, mock, test } from 'claude-code/testing'

const TOOL = 'mcp__self-compact__compact'

/** The engine beneath the mod; `session.compact` refuses its first `busy` calls, as it does while a turn runs. */
function world(on: On, { busy = 0, veto = false } = {}) {
  const clock = mock.clock(on, { now: 0 })
  const seen = { compacts: [] as (string | undefined)[], prompts: [] as unknown[] }
  on('session.start', (_$, e) => ({ cwd: e.cwd }))
  on('turn.complete', (_$, e) => ({ text: e.answer }))
  on('tool.register', (_$, e) => ({ value: { tool: `mcp__self-compact__${e.name}` } }))
  on('session.compact', (_$, e) => {
    if (busy-- > 0) throw new Error('a turn is running')
    seen.compacts.push(e.instructions)
    return veto ? { skip: 'not now' } : { messages: [{ role: 'user', text: 'the summary', toolUses: [] }] }
  })
  on('prompt.submit', (_$, e) => {
    seen.prompts.push({ text: e.text, origin: e.origin })
    return { text: e.text }
  })
  return { clock, seen }
}

const start = ($: Engine) => $.session.start({ cwd: '/w', surface: 'terminal', isInteractive: true })
const ask = ($: Engine, input: object) => $.tool.call({ tool: TOOL, tool_use_id: 'u1', ...input } as never)
const end = ($: Engine, extra = {}) =>
  $.turn.complete({ reason: 'answer', answer: 'done', durationMs: 1, isAborted: false, turnId: 't1', ...extra })
const submitted = (text: string) => ({ text, origin: { kind: 'plugin', name: 'self-compact' } })

test('asked mid-turn, it compacts when the turn ends and then goes on with `then`, framed as its own', async ($, on) => {
  const { clock, seen } = world(on)
  await start($)
  await ask($, { instructions: 'keep the open items', then: 'carry on with step 3' })
  expect(seen.compacts).toEqual([])
  await end($)
  await clock.advance(500)
  expect(seen.compacts).toEqual(['keep the open items'])
  expect(seen.prompts).toEqual([submitted('carry on with step 3')])
})

test('nothing happens at the end of a turn that did not ask', async ($, on) => {
  const { clock, seen } = world(on)
  await start($)
  await end($)
  await clock.advance(10_000)
  expect(seen.compacts).toEqual([])
  expect(seen.prompts).toEqual([])
})

test("a subagent's turn ending does not set it off; the main turn's end does", async ($, on) => {
  const { clock, seen } = world(on)
  await start($)
  await ask($, { then: 'go on' })
  await end($, { agentId: 'a1' })
  await clock.advance(10_000)
  expect(seen.compacts).toEqual([])
  await end($)
  await clock.advance(500)
  expect(seen.compacts).toEqual([undefined])
})

test('a subagent cannot ask', async ($, on) => {
  const { clock, seen } = world(on)
  await start($)
  expect(await ask($, { then: 'go on', agentId: 'a1' })).toEqual({ deny: 'only the main conversation can compact itself' })
  await end($)
  await clock.advance(10_000)
  expect(seen.compacts).toEqual([])
})

test('an interrupted turn drops the ask', async ($, on) => {
  const { clock, seen } = world(on)
  await start($)
  await ask($, { then: 'go on' })
  await end($, { reason: 'aborted', isAborted: true })
  await end($)
  await clock.advance(10_000)
  expect(seen.compacts).toEqual([])
})

test('a vetoed compaction submits nothing', async ($, on) => {
  const { clock, seen } = world(on, { veto: true })
  await start($)
  await ask($, { then: 'go on' })
  await end($)
  await clock.advance(10_000)
  expect(seen.compacts).toEqual([undefined])
  expect(seen.prompts).toEqual([])
})

test('it waits out a still-running turn, and compacts once', async ($, on) => {
  const { clock, seen } = world(on, { busy: 2 })
  await start($)
  await ask($, { then: 'go on' })
  await end($)
  await clock.advance(1_500)
  expect(seen.compacts).toEqual([undefined])
  expect(seen.prompts).toEqual([submitted('go on')])
  await end($)
  await clock.advance(10_000)
  expect(seen.compacts).toHaveLength(1)
})
