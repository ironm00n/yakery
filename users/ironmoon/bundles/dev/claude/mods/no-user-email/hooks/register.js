export function register(on) {
  on('prompt.context', async (_$, e, next) => {
    const context = await next(e)
    return { ...context, blocks: context.blocks.filter(({ name }) => name !== 'userEmail') }
  })
}
