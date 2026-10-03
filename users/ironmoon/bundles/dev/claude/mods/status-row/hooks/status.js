// SGR colors as palette slots: mod trees draw a color name as a fixed hex and refuse `ansi:`
const SGR = { red: 'ansi256(1)', green: 'ansi256(2)', yellow: 'ansi256(3)', blue: 'ansi256(4)', magenta: 'ansi256(5)', cyan: 'ansi256(6)', gray: 'ansi256(8)' }
const SEPARATOR = { text: ' · ', color: SGR.gray }
const WINDOWS = [
  ['five_hour', '5h'],
  ['seven_day', '7d'],
]
const TIMER_SAND = '\u{f051f}'
const TIMER_SAND_EMPTY = '\u{f06ad}'
// the stock engine squeezes the mode label along with a row wider than the room beside it, and the label wraps
const HOLDS_MODE_LABEL = 'prompt_hint_mode'
const FOOTER_PADDING = 4
const WIDEST_MODE_LABEL = '⏵⏵ bypass permissions on · '

/**
 * Columns the row may take beside the mode label: undefined where the engine
 * holds the label at its width (the tap's patch, named in `patches`) or has
 * not measured the terminal.
 */
export const budget = (patches, viewport) =>
  patches?.includes(HOLDS_MODE_LABEL) || viewport === undefined
    ? undefined
    : Math.max(viewport.columns - FOOTER_PADDING - WIDEST_MODE_LABEL.length, 0)

export const heat = (percent) => (percent >= 90 ? SGR.red : percent >= 70 ? SGR.yellow : SGR.green)

export function si(n) {
  if (n >= 1e6) return `${Math.floor(n / 1e5) / 10}m`
  if (n >= 1e3) return `${Math.floor(n / 1e3)}k`
  return `${n}`
}

export function countdown(ms) {
  const minutes = Math.ceil(Math.max(ms, 0) / 60_000)
  const parts = [
    [Math.floor(minutes / 1440), 'd'],
    [Math.floor(minutes / 60) % 24, 'h'],
    [minutes % 60, 'm'],
  ]
  while (parts.length > 1 && parts[0][0] === 0) parts.shift()
  return parts
    .slice(0, 2)
    .map(([n, unit]) => `${n}${unit}`)
    .join('')
}

const under = (path, dir) => path.startsWith(`${dir}/`)
const tilde = (path, home) => (path === home || under(path, home) ? `~${path.slice(home.length)}` : path)

function dirs({ home, root, cwd }) {
  const here = under(cwd, root) ? `./${cwd.slice(root.length + 1)}` : tilde(cwd, home)
  return [{ text: tilde(root, home), color: SGR.blue }, ...(cwd === root ? [] : [{ text: ` ${here}`, color: SGR.cyan }])]
}

function model({ model, effort }) {
  if (!model) return []
  const name = model.replace(/^claude-/, '')
  return [{ text: effort === undefined ? name : `${name} (${effort})`, color: SGR.magenta }]
}

function context({ context }) {
  if (context?.tokens === undefined) return []
  const { tokens, window } = context
  if (!window) return [{ text: si(tokens) }]
  const percent = Math.floor(context.percent ?? (100 * tokens) / window)
  const color = heat(percent)
  return [
    { text: si(tokens), color },
    { text: `/${si(window)} (` },
    { text: `${percent}%`, color },
    { text: ')' },
  ]
}

// only the tap's patched claude-code passes prompt_cache
function cache({ promptCache, now }) {
  if (!promptCache) return []
  const { warm, expires_at } = promptCache
  if (!warm || !expires_at) return [{ text: TIMER_SAND_EMPTY, color: SGR.yellow }]
  return [
    { text: TIMER_SAND, color: SGR.green },
    { text: ` ${countdown(expires_at * 1000 - now)}`, color: SGR.gray },
  ]
}

const windows = ({ rateLimits, now }) =>
  WINDOWS.map(([kind, label]) => {
    const limit = rateLimits.find((l) => l.kind === kind)
    if (!limit) return []
    const percent = Math.floor(limit.percentUsed)
    const reset = limit.resetsAt === undefined ? [] : [{ text: ` ${countdown(Date.parse(limit.resetsAt) - now)}`, color: SGR.gray }]
    return [{ text: `${label} ${percent}%`, color: heat(percent) }, ...reset]
  })

const version = ({ version }) => [{ text: `v${version}` }]

const KEY_ONLY_ACTIONS = ['for shortcuts', 'to interrupt', 'to copy', 'for agents', 'to manage']

const HINT_REWRITES = [
  [/\([^()]* to cycle\)/, ''],
  [new RegExp(`(\\S+) (?:${KEY_ONLY_ACTIONS.join('|')})`, 'g'), '$1'],
]

export const stripHint = (hint) =>
  HINT_REWRITES.reduce((text, [pattern, replacement]) => text.replace(pattern, replacement), hint)
    .split(' · ')
    .map((part) => part.trim())
    .filter(Boolean)
    .join(' · ')

const hint = ({ hint }) => {
  const shown = stripHint(hint)
  return shown ? [{ text: shown }] : []
}

const dimUncolored = (run) => (run.color === undefined ? { ...run, dimColor: true } : run)

/**
 * The row as styled runs, from a snapshot of the session and the agent in
 * view.
 */
export const status = (snapshot) =>
  [dirs(snapshot), model(snapshot), context(snapshot), cache(snapshot), ...windows(snapshot), version(snapshot), hint(snapshot)]
    .filter((runs) => runs.length > 0)
    .flatMap((runs, i) => (i === 0 ? runs : [SEPARATOR, ...runs]))
    .map(dimUncolored)
