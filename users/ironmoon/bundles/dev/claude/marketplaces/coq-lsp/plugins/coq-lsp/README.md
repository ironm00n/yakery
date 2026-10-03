# coq-lsp plugin

Registers [coq-lsp](https://github.com/ejgallego/coq-lsp) as the language server
for `.v` (Coq/Rocq) files.

The server is launched as `direnv exec . coq-lsp`, so it activates the devshell
of the workspace root (the cwd the harness spawns it in) and inherits its
environment (`ROCQPATH`, etc.). coq-lsp then resolves the load path from the
project's `_CoqProject`. Requires the project to have an allowed `.envrc`
(`direnv allow`) that provides `coq-lsp`.
