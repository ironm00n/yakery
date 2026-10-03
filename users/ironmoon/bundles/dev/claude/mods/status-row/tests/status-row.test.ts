import type { On } from 'claude-code'
import { expect, test } from 'claude-code/testing'
import { budget, countdown, heat, si, status, stripHint } from '../hooks/status.js'

const NOW = Date.parse('2026-10-02T20:00:00Z')
const HOME = '/home/u'
const SESSION_MODEL = 'claude-opus-5-5'

const WARM_CACHE = { warm: true, ttl: '1h', expires_at: (NOW + 42 * 60_000) / 1000 }

const RATE_LIMITS = [
  { kind: 'five_hour', percentUsed: 86, resetsAt: '2026-10-02T23:16:00Z' },
  { kind: 'seven_day', percentUsed: 12.5 },
  { kind: 'spend_limit', percentUsed: 3 },
]

const answer =
  <V>(value: V) =>
  () => ({ value })

const STOCK = { version: '2.1.287' }
const VIEWPORT = { columns: 80, rows: 24, isFullscreen: false }

function stubSession(on: On, version: { version: string; patches?: readonly string[] } = STOCK) {
  on(
    'session.usage',
    answer({ startedAt: 0, context: { tokens: 412_000, window: 1_000_000, percent: 41 }, rateLimits: RATE_LIMITS, prompt_cache: WARM_CACHE }),
  )
  on('session.root', answer(`${HOME}/proj`))
  on('session.cwd', answer(`${HOME}/proj/sub`))
  on('env.get', answer(HOME))
  on('session.model', answer(SESSION_MODEL))
  on('session.version', answer(version))
  on('clock.now', answer(NOW))
  on('ui.render', () => ({ type: 'Text', props: {}, children: [] }))
}

function stubStep(on: On, tokens: { input: number; cached: number; written: number }) {
  on('turn.step', async function* (_$, e) {
    return {
      turnId: e.turnId,
      index: e.index,
      answer: '',
      toolUses: [],
      stopReason: 'end_turn',
      usage: {
        model: e.model,
        input_tokens: tokens.input,
        cache_read_input_tokens: tokens.cached,
        cache_creation_input_tokens: tokens.written,
        output_tokens: 10,
      },
    }
  })
}

const view = (agentId?: string) =>
  ({
    plugin: 'status-row',
    surface: 'terminal',
    component: 'AbovePrompt',
    props: {
      hasSurvey: false,
      isWorking: false,
      maxRows: 10,
      bodyColumns: 120,
      scroll: { offset: 0, bodyRows: 10 },
      view: agentId === undefined ? {} : { agentId },
    },
  }) as const

const hintRow = (hint: string, viewport?: typeof VIEWPORT) =>
  ({
    plugin: 'status-row',
    surface: 'terminal',
    component: 'PromptHint',
    ...(viewport && { viewport }),
    props: { isDraft: false, isWorking: true, hint },
  }) as const

const text = (runs: { text: string }[]) => runs.map((r) => r.text).join('')

const SNAPSHOT = {
  home: HOME,
  root: `${HOME}/proj`,
  cwd: `${HOME}/proj/sub`,
  model: SESSION_MODEL,
  effort: 'xhigh',
  context: { tokens: 412_000, window: 1_000_000, percent: 41 },
  promptCache: WARM_CACHE,
  rateLimits: RATE_LIMITS,
  version: '2.1.287',
  hint: 'esc to interrupt',
  now: NOW,
}

test('the row reads dirs · model · ctx · cache · windows · version · hint', () => {
  const runs = status(SNAPSHOT)
  expect(text(runs)).toBe('~/proj ./sub · opus-5-5 (xhigh) · 412k/1m (41%) · \u{f051f} 42m · 5h 86% 3h16m · 7d 12% · v2.1.287 · esc')
  expect(runs.find((r) => r.text === '\u{f051f}')).toMatchObject({ color: 'ansi256(2)' })
  expect(runs.find((r) => r.text === '5h 86%')).toMatchObject({ color: 'ansi256(3)' })
  expect(runs.find((r) => r.text === '41%')).toMatchObject({ color: 'ansi256(2)' })
  expect(runs.find((r) => r.text === 'esc')).toMatchObject({ dimColor: true })
})

test('each run is a terminal palette slot or dim, so the row follows the terminal theme', () => {
  const isPaletteSlot = (color: string) => /^ansi256\(([0-9]|1[0-5])\)$/.test(color)
  expect(status(SNAPSHOT).filter((r) => (r.color === undefined ? r.dimColor !== true : !isPaletteSlot(r.color)))).toEqual([])
  expect(status(SNAPSHOT).find((r) => r.text === 'v2.1.287')).toMatchObject({ dimColor: true })
})

test('segments without a figure drop out with their separator', () => {
  const runs = status({ home: HOME, root: HOME, cwd: HOME, model: undefined, rateLimits: [], version: '2.1.287', hint: '', now: NOW })
  expect(text(runs)).toBe('~ · v2.1.287')
})

test('a cold cache shows the empty timer and no countdown', () => {
  for (const promptCache of [{ ...WARM_CACHE, warm: false }, { ...WARM_CACHE, expires_at: null }]) {
    const runs = status({ ...SNAPSHOT, promptCache })
    expect(text(runs)).toContain(' · \u{f06ad} · 5h')
    expect(runs.find((r) => r.text === '\u{f06ad}')).toMatchObject({ color: 'ansi256(3)' })
  }
})

test("the engine hint loses its mode-cycle reminder, whatever key cycles", () => {
  expect(stripHint('(shift+tab to cycle) · ← 1 agent')).toBe('← 1 agent')
  expect(stripHint('(ctrl+m to cycle)')).toBe('')
  expect(stripHint('1 shell')).toBe('1 shell')
  expect(text(status({ ...SNAPSHOT, hint: '(shift+tab to cycle)' }))).toMatch(/v2\.1\.287$/)
})

test('the shortcuts, interrupt, copy, agents and manage hints keep only their keys', () => {
  expect(stripHint('? for shortcuts')).toBe('?')
  expect(stripHint('esc to interrupt')).toBe('esc')
  expect(stripHint('ctrl+c to copy')).toBe('ctrl+c')
  expect(stripHint('← for agents')).toBe('←')
  expect(stripHint('↓ to manage')).toBe('↓')
  expect(stripHint('(shift+tab to cycle) · ← for agents')).toBe('←')
  expect(stripHint('esc to interrupt · ← for agents')).toBe('esc · ←')
  expect(stripHint('1 shell · esc to interrupt · ← for agents · ↓ to manage')).toBe('1 shell · esc · ← · ↓')
  expect(stripHint('← to go back')).toBe('← to go back')
})

test('heat thresholds, si units and countdown rounding', () => {
  expect([heat(69), heat(70), heat(89), heat(90)]).toEqual(['ansi256(2)', 'ansi256(3)', 'ansi256(3)', 'ansi256(1)'])
  expect([si(999), si(1_500), si(1_250_000)]).toEqual(['999', '1k', '1.2m'])
  expect(countdown(-5_000)).toBe('0m')
  expect(countdown(61 * 60_000)).toBe('1h1m')
  expect(countdown((2 * 1440 + 3 * 60 + 5) * 60_000)).toBe('2d3h')
  expect(countdown(1)).toBe('1m')
})

test('beside a mode label the engine may squeeze, the row keeps to what the widest label leaves', () => {
  expect(budget(undefined, VIEWPORT)).toBe(80 - 4 - '⏵⏵ bypass permissions on · '.length)
  expect(budget([], { ...VIEWPORT, columns: 20 })).toBe(0)
  expect(budget(['session_cache', 'prompt_hint_mode'], VIEWPORT)).toBeUndefined()
  expect(budget(undefined, undefined)).toBeUndefined()
})

for (const [engine, version, boxed] of [
  ['a stock engine gets the row boxed to its budget', STOCK, true],
  ['an engine that holds the label gets the row bare', { ...STOCK, patches: ['prompt_hint_mode'] }, false],
] as const) {
  test(engine, async ($, on) => {
    stubSession(on, version)
    const row = await $.ui.mount(hintRow('esc to interrupt', VIEWPORT))
    expect((await row.find({ type: 'Box' })) !== undefined).toBe(boxed)
    await row.unmount()
  })
}

test('the main conversation shows the session figures and the engine hint', async ($, on) => {
  stubSession(on)
  await (await $.ui.mount(view())).unmount()
  const row = await $.ui.mount(hintRow('esc to interrupt'))
  for (const shown of ['opus-5-5', '412k', '\u{f051f}', ' 42m', '5h 86%', ' 3h16m', '7d 12%', 'esc']) {
    expect(await row.find({ type: 'Text', text: shown })).toBeDefined()
  }
  expect(await row.find({ type: 'Text', text: '3%' })).toBeUndefined()
  await row.unmount()
})

test("the main conversation's effort is its last step's", async ($, on) => {
  stubSession(on)
  stubStep(on, { input: 1_000, cached: 25_000, written: 4_000 })
  for await (const _ of $.turn.step({ turnId: 't', index: 0, model: SESSION_MODEL, effort: 'xhigh', messageCount: 1 }));

  await (await $.ui.mount(view())).unmount()
  const row = await $.ui.mount(hintRow(''))
  expect(await row.find({ type: 'Text', text: 'opus-5-5 (xhigh)' })).toBeDefined()
  await row.unmount()
})

test('a subagent in view shows its own model and context', async ($, on) => {
  stubSession(on)
  stubStep(on, { input: 1_000, cached: 25_000, written: 4_000 })
  for await (const _ of $.turn.step({ turnId: 't', index: 0, model: 'claude-haiku-4-5', effort: 'low', messageCount: 1, agentId: 'a1' }));

  await (await $.ui.mount(view('a1'))).unmount()
  const row = await $.ui.mount(hintRow(''))
  expect(await row.find({ type: 'Text', text: 'haiku-4-5 (low)' })).toBeDefined()
  expect(await row.find({ type: 'Text', text: '30k' })).toBeDefined()
  expect(await row.find({ type: 'Text', text: /\/1m/ })).toBeUndefined()
  expect(await row.find({ type: 'Text', text: 'opus-5-5' })).toBeUndefined()
  await row.unmount()
})

test("a subagent on the session's model borrows the session's window", async ($, on) => {
  stubSession(on)
  stubStep(on, { input: 1_000, cached: 25_000, written: 4_000 })
  for await (const _ of $.turn.step({ turnId: 't', index: 0, model: SESSION_MODEL, effort: 'high', messageCount: 1, agentId: 'a2' }));

  await (await $.ui.mount(view('a2'))).unmount()
  const row = await $.ui.mount(hintRow(''))
  expect(await row.find({ type: 'Text', text: '/1m (' })).toBeDefined()
  expect(await row.find({ type: 'Text', text: '3%' })).toBeDefined()
  await row.unmount()
})
