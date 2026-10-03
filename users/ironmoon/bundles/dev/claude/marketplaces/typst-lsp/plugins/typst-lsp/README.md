# typst-lsp plugin

Registers [tinymist](https://github.com/Myriad-Dreamin/tinymist) as the language
server for `.typ` (Typst) files.

The server is launched as `direnv exec . tinymist lsp`, so it activates the
devshell of the workspace root (the cwd the harness spawns it in) and inherits
its environment (the `tinymist` binary and fonts). Requires the project to have
an allowed `.envrc` (`direnv allow`) that provides `tinymist` — e.g. this repo's
`use flake ~/playground/devshells/typst`.
