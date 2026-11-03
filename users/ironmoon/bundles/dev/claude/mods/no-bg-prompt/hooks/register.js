export function register(on) {
  on('prompt.section', { name: 'bg-session' }, () => ({ text: null }))
  on('prompt.attachment', { type: 'task_reminder' }, () => ({ text: null }))
}
