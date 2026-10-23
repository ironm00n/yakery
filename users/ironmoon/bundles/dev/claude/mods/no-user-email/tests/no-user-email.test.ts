import { expect, test } from 'claude-code/testing'

const BLOCKS = [
  { name: 'claudeMd', text: 'instructions' },
  { name: 'userEmail', text: "The user's email address is someone@example.com." },
  { name: 'attachedProject', text: 'project' },
  { name: 'currentDate', text: "Today's date is 2026-10-11." },
]

test('the userEmail block is dropped and the other blocks pass through in order', async ($, on) => {
  on('prompt.context', (_$, e) => ({ ...e, blocks: BLOCKS }))
  const context = await $.prompt.context({ blocks: [], instructionFiles: [] })
  expect(context.blocks).toEqual(BLOCKS.filter(({ name }) => name !== 'userEmail'))
})
