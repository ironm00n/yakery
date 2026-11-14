# Claude Code on each account, as root sets it up: its own tap, and what its user manager hands each session. Its
# settings and supervisor are its home's (../home/claude.nix).
{
  imports = [
    ./tap.nix
    ./sessions.nix
  ];
}
