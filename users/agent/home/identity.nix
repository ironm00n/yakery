# Its commits name the agent identity and are signed with the key ../identity.nix has the account make, or the one he
# adopts into every account.
{
  lib,
  pkgs,
  my-lib,
  ...
}:
let
  inherit (my-lib.ironmoon.mail.agents) identity;
  inherit (identity) email;
  gpg = lib.getExe' pkgs.gnupg "gpg";
in
{
  programs.git = {
    enable = true;
    settings.user = identity;
    signing = {
      key = email;
      signByDefault = true;
      format = "openpgp";
      signer = gpg;
    };
  };
  programs.jujutsu = {
    enable = true;
    settings = {
      user = identity;
      signing = {
        behavior = "own";
        backend = "gpg";
        key = email;
        backends.gpg.program = gpg;
      };
    };
  };
}
