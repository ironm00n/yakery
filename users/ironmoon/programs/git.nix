{ pkgs, ... }:
{
  enable = true;
  userName = "IRONM00N";
  userEmail = "me@ironmoon.dev";
  signing = {
    signByDefault = true;
    key = null;
  };
  extraConfig =
    let
      gh-helper = {
        helper = "!${pkgs.gh}/bin/gh auth git-credential";
      };
    in
    {
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
    };
}
