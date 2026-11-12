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
    mkMerge
    mkOption
    types
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
  agentEmail = ironmoon.mail.agents.identity.email;
  # jj.toml's agent identity scope is the one list of repos; git gets the same identity in the same repos
  agentIdentity =
    let
      scope =
        lib.findFirst (s: s ? user) (throw "jj.toml has no identity scope")
          (builtins.fromTOML (builtins.readFile ./jj.toml))."--scope";
    in
    assert lib.assertMsg (
      scope.user == ironmoon.mail.agents.identity
    ) "jj.toml's agent identity isn't ${builtins.toJSON ironmoon.mail.agents.identity}";
    scope;
  owner = cfg.account == "owner";
  # settings.json names paths under Owen's home; another account's copy has its own home in their place
  ownerHome = "/home/ironmoon";
  rehome =
    v:
    if lib.isString v then
      lib.replaceStrings [ "${ownerHome}/" ] [ "${config.home.homeDirectory}/" ] v
    else if lib.isList v then
      map rehome v
    else if lib.isAttrs v then
      lib.mapAttrs (_: rehome) v
    else
      v;
  # a Mythos model runs on his account only, so another account's copy defaults to Opus
  settings =
    if config.home.homeDirectory == ownerHome then
      symlink ./settings.json
    else
      pkgs.writeText "settings.json" (
        builtins.toJSON (rehome (lib.importJSON ./settings.json) // { model = "claude-opus-5-5"; })
      );
  # the agent account is the agent in every repo, so its jj.toml scopes drop their repository lists
  jjEverywhere = (pkgs.formats.toml { }).generate "claude.toml" (
    lib.mapAttrs (
      _: map (scope: scope // { "--when" = removeAttrs scope."--when" [ "repositories" ]; })
    ) (builtins.fromTOML (builtins.readFile ./jj.toml))
  );
in
{
  options.bundles.dev.claude = {
    enable = mkEnableOption "claude code";

    account = mkOption {
      type = types.enum [
        "owner"
        "agent"
      ];
      default = "owner";
      description = ''
        Whose sessions these are: Owen's, with the agent identity in his listed repos and Syncthing's ignore list for
        ~/.claude; or the agent account's, the agent in every repo and no Syncthing.
      '';
    };
  };

  config = mkIf cfg.enable (mkMerge [
    (mkIf owner {
      # a real file, not a link: Syncthing's docs don't say it follows a linked .stignore
      home.activation.stignore = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
        run install -m 644 ${stignore} ${config.home.homeDirectory}/.claude/.stignore
      '';
    })
    {
      home.packages = [
        claude-code
        agent-mail
      ];
      xdg.configFile."agent-mail/config.json".text = builtins.toJSON {
        login = agentEmail;
        password-file = "~/.claude/secrets/agent-ironmoon-dev-password";
      };

      xdg.configFile."jj/conf.d/claude.toml".source = if owner then symlink ./jj.toml else jjEverywhere;
      # included only from Claude sessions, by settings.json's GIT_CONFIG_* env
      xdg.configFile."git/claude-identity".text =
        if owner then
          lib.concatMapStrings (repo: ''
            [includeIf "gitdir:${repo}/"]
            	path = claude-identity.user
          '') agentIdentity."--when".repositories
        else
          "[include]\n\tpath = claude-identity.user\n";
      xdg.configFile."git/claude-identity.user".text = lib.generators.toGitINI {
        inherit (agentIdentity) user;
      };
      # a push from a Claude Code session keeps its commit messages within the width rule (hooks/lib/widths.sh) and the
      # provenance rule (hooks/lib/provenance.sh): jj pushes go through jj-push.sh, and plain git ones run this beside
      # the repo's own pre-push hook, which passes everything outside a session
      programs.git.settings."hook \"commit-rules\"" = {
        event = "pre-push";
        # the generation's own copy, so a checkout of /etc/nixos without the script can't break every push
        command = "${./hooks}/git-pre-push.sh";
      };
      # a lane on an agent account lands notes master by push, and its land can't reach his checkout, so his account
      # runs what land runs after a push, every five minutes
      systemd.user = lib.mkIf owner {
        services.sync-checkout = {
          Unit.Description = "Rebase his notes checkout onto notes master, as tools/land does after a push";
          Service = {
            Type = "oneshot";
            Environment = "PATH=%h/.nix-profile/bin:/etc/profiles/per-user/%u/bin:/run/current-system/sw/bin";
            ExecStart = "%h/workspace/claude/tools/sync-checkout %h/workspace/claude";
          };
        };
        timers.sync-checkout = {
          Unit.Description = "Sync his notes checkout every five minutes";
          Timer.OnCalendar = "*:0/5";
          Install.WantedBy = [ "timers.target" ];
        };
      };

      home.file = {
        # no XDG: https://github.com/anthropics/claude-code/issues/1455
        ".claude/CLAUDE.md".source = symlink ./CLAUDE.md;
        ".claude/settings.json".source = settings;
        ".claude/hooks/".source = symlink ./hooks;
        ".claude/my-marketplaces".source = symlink ./marketplaces;
      }
      // skills;
    }
  ]);
}
