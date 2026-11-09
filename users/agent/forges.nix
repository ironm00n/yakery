# Each account's git pushes and fetches over https sign in to each forge as the agent's own account there, with the
# token he copies into its secrets dir.
{ lib, pkgs, ... }:
let
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
    secrets:
    { username, token }:
    pkgs.writeShellScript "git-credential-${token}" ''
      [ "$1" = get ] || exit 0
      printf 'username=%s\npassword=%s\n' ${username} "$(<"$HOME/${secrets}/${token}")"
    '';
in
{
  home-manager.users = builtins.mapAttrs (_: account: {
    programs.git.settings.credential = lib.mapAttrs (_: forge: {
      helper = "${helper account.secrets forge}";
    }) forges;
  }) (import ./accounts.nix);
}
