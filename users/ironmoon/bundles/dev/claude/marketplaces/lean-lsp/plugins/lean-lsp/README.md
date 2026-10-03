# lean-lsp plugin

Registers the Lean 4 language server (`lake serve`) for `.lean` files.

Unlike `coq-lsp`, the command cannot rely on the cwd the harness spawns it in:
`.lean` files here are reached through the `unboca-theory` symlink from the
`borrow-calculus` workspace root, which is not itself a Lean project, so `lake`
would find no lakefile. The server therefore `cd`s to
`research/gradual-borrows/formalization` and activates that devshell explicitly.
Retarget both that path and `workspaceFolder` to point at a different Lean
project.

Cross-module `textDocument/references` is answered from the `.ilean` index under
`.lake/build/`, so it is only as current as the last `lake build`. Elaboration
of a freshly opened file must finish before position-based queries resolve;
the first request after startup can take a minute.

`diagnostics` is off — with 60 modules the push after every edit is loud. Turn
it on in `marketplace.json` if Lean errors should reach the transcript.
