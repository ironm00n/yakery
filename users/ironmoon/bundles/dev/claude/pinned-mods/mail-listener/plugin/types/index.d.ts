/** Whether this session's mail wait runs, as the mail-listener mod last saw it. */
export type MailListener = { listening: true } | { listening: false; why: string }

declare module 'claude-code' {
  interface PluginState {
    'mail-listener': { listener: MailListener }
  }
}
