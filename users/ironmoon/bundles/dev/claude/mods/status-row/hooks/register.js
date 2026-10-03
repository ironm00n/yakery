import { budget, status } from './status.js'

const COUNTDOWN_MS = 60_000

const AGENT = { plugin: 'status-row', key: 'agent' }
const member = (agentId) => agentId ?? 'main'

let viewed
// a hot reload drops this environment's timer along with the flag
let isTicking = false

const redraw = ($) => $.ui.invalidate('ui.render')

function tick($) {
  if (isTicking) return
  isTicking = true
  $.clock.every(COUNTDOWN_MS, () => redraw($))
}

const contextTokens = (usage) => usage.input_tokens + usage.cache_read_input_tokens + usage.cache_creation_input_tokens

async function snapshot($, hint) {
  const [usage, root, cwd, home, sessionModel, { version, patches }, now, { value: agent }] = await Promise.all([
    $.session.usage(),
    $.session.root(),
    $.session.cwd(),
    $.env.get('HOME'),
    $.session.model(),
    $.session.version(),
    $.clock.now(),
    $.state.get({ ...AGENT, id: member(viewed) }),
  ])
  const view =
    viewed === undefined
      ? { model: sessionModel, effort: agent?.effort, context: usage.context }
      : agent && {
          model: agent.model,
          effort: agent.effort,
          context: { tokens: agent.tokens, window: agent.model === sessionModel ? usage.context.window : undefined },
        }
  return { home, root, cwd, ...view, promptCache: usage.prompt_cache, rateLimits: usage.rateLimits, version, patches, hint, now }
}

export function register(on) {
  on('session.start', async ($, e, next) => {
    tick($)
    return next(e)
  })

  on('session.measure', async ($, e, next) => {
    tick($)
    redraw($)
    return next(e)
  })

  on('turn.step', async function* ($, e, next) {
    const result = yield* next(e)
    tick($)
    if (result.usage) {
      await $.state.set({ ...AGENT, id: member(e.agentId) }, { model: e.model, effort: e.effort, tokens: contextTokens(result.usage) })
    }
    return result
  })

  on('ui.render', { component: 'AbovePrompt' }, async ($, e, next) => {
    if (e.props.view.agentId !== viewed) {
      viewed = e.props.view.agentId
      redraw($)
    }
    return next(e)
  })

  on('ui.render', { component: 'PromptHint' }, async ($, e) => {
    const { Box, Text } = $.ui.resolve(e)
    const shot = await snapshot($, e.props.hint)
    const row = Text({ wrap: 'truncate', children: status(shot).map(({ text, ...style }) => Text({ ...style, children: [text] })) })
    const width = budget(shot.patches, e.viewport)
    return width === undefined ? row : Box({ width, children: [row] })
  })
}
