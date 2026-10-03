/** Where the tap sends this session's turns, as it last answered. */
export type PoolRoute =
  | {
      /** The session's switch over the tap's session-gated rules. */
      switch: 'on' | 'off'
      /** The rule that takes a Claude model's turn from the session, or null for its own account. */
      through: string | null
      /** Whether this tap has carried any request of the session. */
      seen: boolean
    }
  | {
      /** Why no tap answers for this session. */
      unavailable: string
    }

declare module 'claude-code' {
  interface PluginState {
    pool: { route: PoolRoute }
  }
}
