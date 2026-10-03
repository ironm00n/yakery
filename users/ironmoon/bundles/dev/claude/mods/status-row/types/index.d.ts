export type StatusRowAgent = {
  model: string
  effort?: string | number
  tokens: number
}

declare module 'claude-code' {
  interface PluginState {
    'status-row': { agent: StateFamily<StatusRowAgent> }
  }
}
