import { tail } from './tail.js'

const ROWS = 8
const INDENT = 2
const MAIN = 'main'

const live = new Map()

function append(agent, { index, text }) {
  const block = live.get(agent)
  live.set(agent, block?.index === index ? { index, text: block.text + text } : { index, text })
}

const spinnerAgent = async ($, requestId) => (requestId === (await $.session.id()) ? MAIN : requestId)

export function register(on) {
  on('turn.step', async function* ($, e, next) {
    const agent = e.agentId ?? MAIN
    try {
      for await (const chunk of next(e)) {
        if (chunk.kind === 'thinking') {
          append(agent, chunk)
          $.ui.invalidate('ui.render')
        } else if (chunk.kind !== 'engine' && live.delete(agent)) {
          $.ui.invalidate('ui.render')
        }
        yield chunk
      }
    } finally {
      if (live.delete(agent)) $.ui.invalidate('ui.render')
    }
  })

  on('ui.render', { component: 'Spinner' }, async ($, e, next) => {
    const block = live.get(await spinnerAgent($, e.requestId))
    if (!block) return next(e)
    const { Box, Text } = $.ui.resolve(e)
    const width = (e.viewport?.columns ?? 80) - INDENT
    return Box({
      flexDirection: 'column',
      children: [
        await next(e),
        Box({
          paddingLeft: INDENT,
          children: [Text({ dimColor: true, italic: true, children: [tail(block.text, width, ROWS)] })],
        }),
      ],
    })
  })
}
