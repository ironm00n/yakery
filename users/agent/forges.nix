# Its git pushes and fetches over https sign in to each forge as the agent's own account there, with the token he copies
# into its secrets dir.
{ lib, pkgs, ... }:
let
  inherit (import ./account.nix) name secrets;
  forges = {
    "https://code.ironmoon.dev" = {
      username = "agent";
      token = "forgejo-agent-pat";
    };
    "https://github.com" = {
      username = "ironmoon-agent";
      token = "gh-agent-pat";
    };
  };
  helper =
    { username, token }:
    pkgs.writeShellScript "git-credential-${token}" ''
      [ "$1" = get ] || exit 0
      printf 'username=%s\npassword=%s\n' ${username} "$(<"$HOME/${secrets}/${token}")"
    '';
in
{
  home-manager.users.${name}.programs.git.settings.credential = lib.mapAttrs (_: forge: {
    helper = "${helper forge}";
  }) forges;
}
