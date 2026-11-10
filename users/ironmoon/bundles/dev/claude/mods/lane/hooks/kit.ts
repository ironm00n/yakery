/** What a lane's session is handed when its context starts over, each part one block of text. */

export type Item = {
  number: number
  title: string
  url: string
  updated: string
  labels: string[]
  body: string
  newest: { by: string; at: string; body: string } | null
}

/** One message to the session, as hooks/messages.jq prints it: its text cut to CAPS.message, `more` the characters cut. */
export type Said = { at: string; from: string; text: string; more: number }
/** The newest messages of one kind a section could show, and how many older ones messages.jq left out. */
export type Heard = { kept: Said[]; older: number }

export type Task = { type: string; description: string; command?: string }
export type Cron = { schedule: string; recurring: boolean; prompt: string }

/** Characters, as the engine caps a prompt's context: 100,000 an entry, 200,000 together. */
export const CAPS = { state: 32_000, carried: 48_000, items: 32_000, itemText: 1_500, owen: 16_000, peers: 12_000, message: 6_000 } as const

export const ownerLabels = (name: string) => [`owner/${name}`, `owner:${name}`]

const marked = (text: string, more: number) => (more === 0 ? text : `${text}… [${more} more characters]`)
const clip = (text: string, cap: number) => marked(text.slice(0, cap), Math.max(text.length - cap, 0))

/** The first of `blocks` whose total stays within `cap`, how many are left out, and how much of `cap` is left. */
function within(blocks: string[], cap: number): { kept: string[]; dropped: number; left: number } {
  const ends = blocks.reduce<number[]>((sums, block) => [...sums, (sums.at(-1) ?? 0) + block.length], [])
  const kept = blocks.slice(0, ends.filter((end) => end <= cap).length)
  return { kept, dropped: blocks.length - kept.length, left: cap - (ends[kept.length - 1] ?? 0) }
}

/** The `carry:` list of a state file's front matter: other files loaded with it. */
export function carried(text: string): string[] {
  const front = /^---\n([\s\S]*?)\n---\n/.exec(text)?.[1] ?? ''
  const list = /^carry:\n((?:[ \t]*- .*\n?)*)/m.exec(front)?.[1] ?? ''
  return [...list.matchAll(/^[ \t]*- (.+?)\s*$/gm)].map(([, path]) => path)
}

export function stateBlock(link: string, path: string, text: string): string {
  const head = `Your state file is ${path} (${link} names it), as it read when this context began. It is what the next context starts from: keep it current, and edit this copy.`
  if (text.length <= CAPS.state) return `${head}\n\n${text}`
  const line = text.lastIndexOf('\n', CAPS.state)
  const cut = line > CAPS.state / 2 ? line : CAPS.state
  return `${head}\n\n${text.slice(0, cut)}\n\n[Cut here: the file is ${text.length} characters, over the ${CAPS.state} this kit loads. Trim it now, moving gathered facts and logs to a file it names, so the next context gets all of it.]`
}

export const unreadStateBlock = (path: string, why: string) =>
  `Your state file ${path} could not be read (${why}). Read it yourself before anything else; it is what this context starts from.`

export const noStateBlock = (name: string, link: string) =>
  `This session, ${name}, has no state file: ${link} leads nowhere. Make one before anything else with \`just lane-state ${name}\` in ~/workspace/claude, then fill in its Goal and Done from what you were asked. It is what the next context starts from.`

export function carriedBlocks(files: { path: string; text: string | null; why?: string }[]): string[] {
  let left: number = CAPS.carried
  return files.map(({ path, text, why }) => {
    if (text === null) return `${path}, which the state file carries, could not be read (${why}). Read it before relying on it.`
    const isCut = text.length > left
    const shown = clip(text, Math.max(left, 0))
    left -= text.length
    return `${path}, which the state file carries, as it read when this context began${isCut ? ' (cut: read the rest with Read)' : ''}:\n\n${shown}`
  })
}

function itemText({ number, title, url, updated, labels, body, newest }: Item): string {
  const comment = newest === null ? 'No comments.' : `Newest comment, by ${newest.by} at ${newest.at}:\n${clip(newest.body, CAPS.itemText)}`
  return `### #${number} ${title}\n${url} · updated ${updated} · ${labels.join(', ')}\n\n${clip(body, CAPS.itemText)}\n\n${comment}`
}

export function itemsBlock(name: string, items: Item[]): string {
  const [slash, colon] = ownerLabels(name)
  if (items.length === 0) return `No open agents/work issue carries ${slash} or ${colon}.`
  const full = within(items.map(itemText), CAPS.items)
  const titles = within(items.slice(full.kept.length).map(({ number, title, url }) => `- #${number} ${title} ${url}`), full.left)
  const listed = titles.kept.length === 0 ? '' : `\n\nAlso open, less recently updated, without their text:\n${titles.kept.join('\n')}`
  const unlisted = titles.dropped === 0 ? '' : `\n\n[${titles.dropped} more are open, the least recently updated; list them on the tracker.]`
  return `The open agents/work issues given to ${name} (${slash} or ${colon}), most recently updated first, each with its newest comment:\n\n${full.kept.join('\n\n')}${listed}${unlisted}`
}

export const unreadItemsBlock = (name: string, why: string) =>
  `The open agents/work issues given to ${name} could not be read (${why}). List them yourself before you pick up work.`

const minute = (iso: string) => `${iso.slice(0, 10)} ${iso.slice(11, 16)}Z`

/** The newest of `heard` within `cap`, oldest first, each message cut so one long report can't push out the rest. */
function newest(heard: Heard, cap: number, where: string): string {
  const newestFirst = heard.kept.map(({ at, from, text, more }) => `[${minute(at)}, ${from}]\n${marked(text, more)}`).reverse()
  const { kept, dropped } = within(newestFirst, cap)
  const older = dropped + heard.older
  const more = older === 0 ? '' : `\n\n[${older} older ones are in ${where}.]`
  return `${kept.length === 0 ? '(none)' : kept.reverse().join('\n\n')}${more}`
}

export function saidBlock({ owen, peers }: { owen: Heard; peers: Heard }, where: string): string {
  return [
    'The newest messages this session was sent, verbatim and oldest first, under the time each was sent (UTC).',
    `From Owen:\n\n${newest(owen, CAPS.owen, where)}`,
    `From other sessions:\n\n${newest(peers, CAPS.peers, where)}`,
  ].join('\n\n')
}

export const unreadSaidBlock = (where: string, why: string) =>
  `The messages this session was sent could not be read (${why}); they are in ${where}.`

const MAIL_WAIT = /(^|[\s;&|(/])agent-mail\s+wait\b/
const LANDS = /(^|[\s;&|(])(just\s+land(\s|$)|(\S*\/)?tools\/land(\s|$)|jj\s+git\s+push\b|git\s+(-[Cc]\s+\S+\s+)*push\b)/
const DRY_RUN = /push\b[^;&|]*\s(--dry-run|-n)(\s|$)/

/** Whether a Bash command lands or pushes work. */
export const lands = (command: string) => LANDS.test(command) && !DRY_RUN.test(command)

/** Background work that a fresh context would see report with no memory of why it started. */
export const inFlight = (tasks: Task[]) => tasks.filter(({ command }) => command === undefined || !MAIL_WAIT.test(command))

export function liveBlock(tasks: Task[], crons: Cron[]): string {
  const lines = tasks.map(({ type, description, command }) => `- ${type}: ${command ?? description}`)
  const running = `Running in the background when the last turn ended (anything started since is not listed):\n${lines.length === 0 ? '- nothing' : lines.join('\n')}`
  const wakes = crons.map(({ schedule, recurring, prompt }) => `- \`${schedule}\`${recurring ? '' : ' (once)'}: ${prompt}`)
  return wakes.length === 0 ? running : `${running}\n\nSet to wake this session then, each with the prompt it submits (CronList shows which are still set):\n${wakes.join('\n')}`
}

export const pointerBlock = (transcript: string) =>
  `This session's whole transcript, from before every compaction, is ${transcript}; recall searches it.`

/** `busy` is what still runs, or undefined where that isn't known yet. */
export function askBlock(tokens: number, why: 'unit' | 'mark', busy: Task[] | undefined, hasState: boolean): string {
  const reason =
    why === 'unit'
      ? `You landed work this turn and the context holds ${Math.round(tokens / 1000)}k tokens`
      : `The context holds ${Math.round(tokens / 1000)}k tokens`
  const running =
    busy === undefined
      ? ' If anything you started still runs, say in the state file what each is for and what to do with its result; it goes on running across the clear, and the next context is told of it.'
      : busy.length === 0
        ? ''
        : ` Still running: ${busy.map(({ description }) => description).join('; ')}; say in the state file what each is for and what to do with its result.`
  const state = hasState ? 'Bring your state file up to date (Status, Plan, Promised, open questions)' : 'Make your state file (see above) and fill it in'
  return `${reason}, so this context is due to start over. ${state}, so a fresh context can carry on from it with nothing else.${running} Then call the rotate tool and end your turn. If you are partway through a step that the file cannot carry, finish the step first.`
}

export const CARRY_ON =
  'This context started over at a rotation. Carry on from your state file and the blocks just before this message: they are all that came across.'

export const carryOnWithout = (link: string, why: string) =>
  `This context started over at a rotation, but the kit could not be handed over (${why}). Read your state file (${link} names it) and carry on from it.`

export const notRotated = (why: string) => `The rotation did not happen (${why}). This context carries on as it was.`

export const notCarried = (why: string, link: string | undefined) =>
  `The lane kit was not carried across this compaction (${why}).${link === undefined ? '' : ` Read your state file (${link} names it) before going on.`}`
