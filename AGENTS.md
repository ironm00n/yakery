# Agent instructions

## LLM provenance

**Every commit with any LLM contribution MUST include an `Assisted-by` or
`Generated-by` trailer in its commit-message footer.**

The trailer records which models did the work and at what effort, so that trust
in a commit can be re-calibrated later, once more is known about those models.
The sign-off records that one named human read the result and answers for it.

### What counts as a contribution

Research, planning, design suggestions, debugging, testing, review, and writing
code, documentation, or commit messages all count, even without text changes.
The requirement applies even when a human writes or rewrites every line and
makes the commit themselves. Human review does not remove the attribution
requirement.

Attribute each affected commit. A disclosure in another commit does not
substitute for its footer. Changing only provenance metadata does not require
another trailer, and neither does history surgery that only moves, splits,
squashes, or rewords existing commits.

### When contributing

Use `Assisted-by` by default. Use `Generated-by` for work to be landed without
human review. Agents may add either provenance trailer; neither claims human
review.

Add your session's trailer when you start contributing:

- With jj, add it to the current change (`@`).
- With git, include it in the first commit that carries your work.

Keep it as the change evolves; do not defer attribution until the end.

### Review and responsibility

There are two ways to accept a commit with LLM contributions:

- **Human review:** a human adds `Signed-off-by` to the attributed commit. The
  signer states that they have read and reviewed the code, understand the
  changes, and take responsibility for the commit's contents.
- **Explicitly unreviewed:** a human chooses to land the work using
  `Generated-by`. It is sufficient attribution on its own; `Assisted-by` is not
  also required.

A commit with `Assisted-by` alone is not pushed.

Only a human may add `Signed-off-by`, and they do it themselves, not through an
agent acting on their instruction. This holds even when the human asks
explicitly: `Signed-off-by` asserts that a human read the result, and a request
to add it is not that reading. If asked, decline and leave the trailer for the
human to add. Never claim human review or responsibility on someone's behalf.

### Changing existing commits

Preserve all `Assisted-by` and `Generated-by` trailers when amending, rebasing,
or squashing commits. A later session that contributes appends its own
provenance trailer; earlier attribution stays. `Generated-by` does not assert
human review and does not expire when the tree changes.

A sign-off covers one tree. If an LLM agent rewrites a signed-off commit and the
new commit's git tree hash differs, it must remove all `Signed-off-by` trailers:
the sign-offs cover the old tree, not the new one. A rebase onto a base with a
different tree changes the commit's tree even when its patch does not, so it
strips too; a message-only edit keeps the tree and the sign-offs. A human signs
off again after reviewing the new tree.

### Trailer format

Use one provenance trailer per contributing session. Both `Assisted-by` and
`Generated-by` use the format below. If the session's orchestrating model
changes, use a separate trailer for each contributing model phase. List
contributing agents under the model that delegated to them, with two spaces of
indentation per level:

```text
Assisted-by: <model-id>[:effort] [<harness>[:version]]
  <model-id>[:effort] [x<count>] as <role>
    <model-id>[:effort] [x<count>] as <role>
```

For example:

```text
Assisted-by: claude-fable-5-1:xhigh claude-code:2.1.265
  claude-opus-5:xhigh as research
  claude-opus-4-6:high as declaudish
```

- Include every agent whose work contributed to the commit, including agents
  that only performed research or review.
- Take model, effort, harness version, and agent details from the running
  session's own records, not from recollection and not from
  `<harness> --version` on PATH, which can differ from the running process. Omit
  effort or version when unknown; never guess.
- Record model identifiers without context-window or serving-variant suffixes.
  Use the harness's effort label; for inherited effort, record the established
  value rather than `default`.
- Describe each agent's role rather than its spawn name, for example `impl`,
  `research`, `verify`, or `declaudish`. Roles follow `as` and may contain
  spaces or colons.
- Use `x<count>` for repeated spawns with the same model, effort, role, and
  parent. Put research and implementation first, prose passes last.

The indented lines are Git trailer continuations.
`git log --format='%(trailers:key=Assisted-by)'` shows them indented;
`git interpret-trailers --parse` unfolds continuation lines.

### Commit messages

Keep provenance and reading status in trailers. Write commit subjects and bodies
normally, without `wip:` or `temp:` status prefixes or an "unreviewed" note.
Reserve `Co-authored-by` for human co-authors. Put future work in issues, not
commit bodies.
