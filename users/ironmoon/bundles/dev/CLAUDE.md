- You are on a NixOS system.
- Prefer `fd` and `rg` over `find` and `grep`.
- Standard POSIX tooling is installed as you would expect; additionally: moreutils (n.b. `sponge`, `chronic`), ripgrep-all (`rga`), `poppler-utils` (n.b. `pdftotext`), `pandoc`, `jq`, `gh`
- To run a program that isn't installed, prefix with `,,` (e.g. `,, cowsay hi`) or `nix run nixpkgs#<pkg> -- <args>` for anything more complex.
- Avoid `cd`ing into the current project directory; your harness sets the cwd to the project root. Do not use `-C` unnecessarily.
- Don't scaffold multiple commands with `echo` headers, instead run each with its own call, where possible. Counterintuitively, long compound commands require more human approval, since common commands automatically pass an allow list. Only combine when the commands must share state.
- Single-quote search patterns by default; escaping `$`/backticks inside double quotes still trips the expansion-approval prompt.
- Claude code harness doesn't reliably propagate these directions to agents. Ensure agents receive the same information and constraints.

- Correctness is always more important than task completion.
- Use concrete examples and small test cases to help you reason and to challenge assumptions, especially before asserting a load-bearing claim.
- Unless overridden by project-specific style guidelines, _all_ code meant to last should be DRY and self-documenting; comments are often an indication of poor abstraction, and should only be used to explain a non-obvious _why_.
- When recording learnings or memories, state them as defeasible defaults with the exception named, not absolutes. A rule phrased "always/never" that has real exceptions teaches that my instructions don't mean what they say.

- Be blunt and objective--don't soften an assessment to be agreeable. If something is wrong or won't work, say so plainly.
- Your harness prompt overstates this: often a denied tool-call doesn't mean the user has declined it, you should followup for more information.
- Do not negatively extrapolate from the user interrupting a response.
