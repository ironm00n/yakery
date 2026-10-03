import type { On } from 'claude-code'
import { expect, test, type Mounted } from 'claude-code/testing'
import { tail } from '../hooks/tail.js'

const spinner = (requestId: string) =>
  ({
    plugin: 'stream-thinking',
    surface: 'terminal',
    component: 'Spinner',
    requestId,
    viewport: { columns: 40, rows: 20 },
    props: { word: 'Sauteing', message: null, suffix: '…', mode: 'thinking' },
  }) as const

const SESSION_ID = 'session-1'
const main = spinner(SESSION_ID)

const CHUNKS = [
  { kind: 'thinking', index: 0, text: 'weighing ' },
  { kind: 'thinking', index: 0, text: 'options' },
  { kind: 'text', index: 1, text: 'done' },
] as const

function stubEngine(on: On) {
  on('session.id', () => ({ value: SESSION_ID }))
  on('ui.render', () => ({ type: 'Text', props: {}, children: ['engine spinner'] }))
  on('turn.step', async function* (_$, e) {
    yield* CHUNKS
    return { turnId: e.turnId, index: e.index, answer: 'done', toolUses: [], stopReason: 'end_turn', usage: null }
  })
}

const step = (agentId?: string) => ({ turnId: 't', index: 0, model: 'claude-test', messageCount: 1, ...(agentId && { agentId }) })

const thinkingIn = (ui: Mounted<'terminal', 'Spinner'>) => ui.find({ type: 'Text', text: 'weighing options' })

test('thinking streams under the spinner until the answer starts', async ($, on) => {
  stubEngine(on)
  const stream = $.turn.step(step())
  const seen = [(await stream.next()).value, (await stream.next()).value]

  const thinking = await $.ui.mount(main)
  expect(await thinkingIn(thinking)).toBeDefined()
  expect(await thinking.find({ type: 'Text', text: 'engine spinner' })).toBeDefined()
  await thinking.unmount()

  seen.push((await stream.next()).value)
  const answering = await $.ui.mount(main)
  expect(await thinkingIn(answering)).toBeUndefined()
  await answering.unmount()

  const end = await stream.next()
  expect(end.done).toBe(true)
  expect(seen).toEqual([...CHUNKS])
  expect(end.value).toMatchObject({ answer: 'done', stopReason: 'end_turn' })
})

test("the engine's fallback spinner id 'main' is the main agent too", async ($, on) => {
  stubEngine(on)
  const stream = $.turn.step(step())
  await stream.next()
  await stream.next()

  const ui = await $.ui.mount(spinner('main'))
  expect(await thinkingIn(ui)).toBeDefined()
  await ui.unmount()
})

test('an abandoned step clears its thinking', async ($, on) => {
  stubEngine(on)
  for await (const _ of $.turn.step(step())) break

  const ui = await $.ui.mount(main)
  expect(await ui.find({ type: 'Text', text: /weighing/ })).toBeUndefined()
  await ui.unmount()
})

test("a subagent's thinking shows under its own spinner only", async ($, on) => {
  stubEngine(on)
  const stream = $.turn.step(step('a1'))
  await stream.next()
  await stream.next()

  const mainUi = await $.ui.mount(main)
  expect(await thinkingIn(mainUi)).toBeUndefined()
  await mainUi.unmount()

  const sub = await $.ui.mount(spinner('a1'))
  expect(await thinkingIn(sub)).toBeDefined()
  await sub.unmount()
})

test('tail keeps the last rows, wrapped at word boundaries', () => {
  expect(tail('one two three four', 9, 8)).toBe('one two\nthree\nfour')
  expect(tail('a\nb\nc\nd', 80, 2)).toBe('c\nd')
  expect(tail('abcdefghij', 4, 8)).toBe('abcd\nefgh\nij')
})
