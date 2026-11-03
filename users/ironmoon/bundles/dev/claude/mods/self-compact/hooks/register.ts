import type { EngineInterface, Register } from 'claude-code'

const TOOL = 'compact'
const WAIT_MS = 500
const TRIES = 20

type Ask = { instructions?: string; then: string }

let asked: Ask | undefined

export const register: Register = (on) => {
  on('session.start', async ($, e, next) => {
    const started = await next(e)
    await $.tool.register({
      name: TOOL,
      description:
        'Compact this conversation when the current turn ends, then go on with `then` as the next prompt. ' +
        '`instructions` says what the summary must keep. Use it when the context is long and the work is at a point a ' +
        'summary can carry, and end the turn right after calling it.',
      inputSchema: {
        type: 'object',
        properties: { instructions: { type: 'string' }, then: { type: 'string' } },
        required: ['then'],
      },
      isDeferred: false,
    })
    return started
  })

  on('tool.call', { tool: `mcp__self-compact__${TOOL}` }, (_$, e) => {
    if (e.agentId !== undefined) return { deny: 'only the main conversation can compact itself' }
    asked = { instructions: e.instructions as string | undefined, then: e.then as string }
    return { result: 'compaction is queued for the end of this turn; end the turn now' }
  })

  on('turn.complete', async ($, e, next) => {
    const done = await next(e)
    if (e.agentId === undefined) {
      const ask = asked
      asked = undefined
      if (ask && e.reason === 'answer') void compact($, ask)
    }
    return done
  })
}

/** `session.compact` rejects while a turn runs, which the engine may still count the one just completed as. */
async function compact($: EngineInterface, { instructions, then }: Ask) {
  for (let i = 1; i <= TRIES; i++) {
    await $.clock.sleep(WAIT_MS)
    try {
      const { skip } = await $.session.compact(instructions ? { instructions } : {})
      if (skip) return void $.ui.log(`self-compact: a hook vetoed the compaction: ${skip}`)
      await $.prompt.submit({ text: then })
      return
    } catch (error) {
      if (i === TRIES) $.ui.log(`self-compact: compaction did not run: ${error}`)
    }
  }
}
