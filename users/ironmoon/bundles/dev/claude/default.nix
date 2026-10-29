{
  config,
  lib,
  pkgs,
  inputs,
  my-lib,
  my-utils,
  pkgs-master,
  ...
}:
let
  inherit (lib)
    mkEnableOption
    mkIf
    mapAttrs'
    nameValuePair
    filterAttrs
    ;
  inherit (my-utils) symlink;
  inherit (pkgs-master) claude-code;
  cfg = config.bundles.dev.claude;
  inherit (my-lib) ironmoon;
  agent-mail = import ../../../../../crates/agent-mail/deployed.nix {
    inherit (pkgs) callPackage;
    inherit my-lib;
    inherit (inputs) secrets;
  };

  # claude code loads any ~/.claude/skills/<name> holding a .claude-plugin/ as a plugin
  skills =
    mapAttrs' (
      name: _: nameValuePair ".claude/skills/${name}" { source = symlink (./mods + "/${name}"); }
    ) (filterAttrs (_: type: type == "directory") (builtins.readDir ./mods))
    // {
      # the tap's profile entry ships the mail-listener its Claude Code trusts, so this reaches exactly that build
      ".claude/skills/mail-listener".source =
        config.lib.file.mkOutOfStoreSymlink "${config.home.homeDirectory}/.nix-profile/share/claude/trusted-prompt-mod";
    };
  # Syncthing shares ~/.claude, and each of these links points into this host's own store
  stignore = pkgs.writeText "stignore" (
    "// home-manager rewrites this file at every switch\n"
    + lib.concatMapStrings (path: "/${lib.removePrefix ".claude/" path}\n") (lib.attrNames skills)
    + "!stignore\n#include stignore\n"
  );
in
{
  options.bundles.dev.claude = {
    enable = mkEnableOption "claude code";
  };

  config = mkIf cfg.enable {
    home.packages = [
      claude-code
      agent-mail
    ];

    xdg.configFile."agent-mail/config.json".text = builtins.toJSON {
      login = "${ironmoon.mail.agents.sharedLocalPart}@${ironmoon.domain}";
      password-file = "~/.claude/secrets/agent-ironmoon-dev-password";
    };

    home.file = {
      # no XDG: https://github.com/anthropics/claude-code/issues/1455
      ".claude/CLAUDE.md".source = symlink ./CLAUDE.md;
      ".claude/settings.json".source = symlink ./settings.json;
      ".claude/hooks/".source = symlink ./hooks;
      ".claude/my-marketplaces".source = symlink ./marketplaces;
    }
    // skills;

    # a real file, not a link: Syncthing's docs don't say it follows a linked .stignore
    home.activation.stignore = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
      run install -m 644 ${stignore} ${config.home.homeDirectory}/.claude/.stignore
    '';
  };
}
