import { expect, test } from 'claude-code/testing'

const BACKGROUND = 'Use `$CLAUDE_JOB_DIR/tmp` for any temporary files (scripts, query files, intermediate outputs) instead of `/tmp`.'
const NUDGE = "The task tools haven't been used recently."

test('the bg-session section is dropped', async ($, on) => {
  on('prompt.section', (_$, e) => ({ text: e.text }))
  expect(await $.prompt.section({ name: 'bg-session', text: BACKGROUND })).toEqual({ text: null })
})

test('every other section passes through', async ($, on) => {
  on('prompt.section', (_$, e) => ({ text: e.text }))
  expect(await $.prompt.section({ name: 'context_management', text: BACKGROUND })).toEqual({ text: BACKGROUND })
})

test('the task-tools reminder is dropped', async ($, on) => {
  on('prompt.attachment', (_$, e) => ({ text: e.text }))
  expect(await $.prompt.attachment({ type: 'task_reminder', text: NUDGE })).toEqual({ text: null })
})

test('every other attachment passes through', async ($, on) => {
  on('prompt.attachment', (_$, e) => ({ text: e.text }))
  expect(await $.prompt.attachment({ type: 'todo_reminder', text: NUDGE })).toEqual({ text: NUDGE })
})
