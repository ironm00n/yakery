{ pkgs, ... }:
{
  enable = true;
  signing = {
    signByDefault = true;
    key = null;
    format = "openpgp";
  };
  settings =
    let
      gh-helper = {
        helper = "!${pkgs.gh}/bin/gh auth git-credential";
      };
    in
    {
      user.name = "IRONM00N";
      user.email = "me@ironmoon.dev";
      "credential \"https://github.com\"" = gh-helper;
      "credential \"https://gist.github.com\"" = gh-helper;
      "credential \"https://github.khoury.northeastern.edu\"" = gh-helper;
      "credential \"https://gist.github.khoury.northeastern.edu\"" = gh-helper;
      pull = {
        rebase = "true";
      };
      push = {
        autoSetupRemote = "true";
        recurseSubmodules = "on-demand";
      };
      init = {
        defaultBranch = "master";
      };
      rebase = {
        autoStash = "true";
      };
      submodule.recurse = "true";
      alias = {
        lg = "log --oneline --graph --decorate --all";
      };
      diff = {
        colorMoved = "default";
      };
      format = {
        pretty = "fuller";
      };
    };
}
