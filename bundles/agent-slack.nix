{
  config,
  lib,
  pkgs,
  inputs,
  ...
}:
let
  inherit (lib)
    genAttrs
    getExe'
    literalExpression
    mkEnableOption
    mkIf
    mkOption
    types
    ;
  cfg = config.bundles.agent-slack;
  agent-slack = pkgs.callPackage ../crates/agent-slack/package.nix { };
  list = "/etc/agent-slack/channels";
  names = [
    "bot_token"
    "app_token"
  ];
  secret = token: "slack/${token}";
in
{
  options.bundles.agent-slack = {
    enable = mkEnableOption "agent-slackd, which holds Slack's Socket Mode connection and an inbox for one agent session, and `agent-slack`, the session's way to it";

    user = mkOption {
      type = types.str;
      default = "ironmoon";
      description = "The one user whose sessions reach agent-slackd's socket.";
    };

    channel = mkOption {
      type = types.str;
      default = "C0CCD4YGE2Y";
      description = "The channel ${list} starts with when it is missing; after that the file is root's to edit, not Nix's.";
    };

    sopsFile = mkOption {
      type = types.nullOr types.path;
      default = "${inputs.secrets}/secrets/slack.yaml";
      defaultText = literalExpression "\"\${inputs.secrets}/secrets/slack.yaml\"";
      description = "The sops file holding bot_token and app_token; null leaves the token files to `tokens`.";
    };

    tokens = mkOption {
      type = types.submodule { options = genAttrs names (_: mkOption { type = types.path; }); };
      default = genAttrs names (name: config.sops.secrets.${secret name}.path);
      defaultText = literalExpression "genAttrs [ \"bot_token\" \"app_token\" ] (name: config.sops.secrets.\"slack/\${name}\".path)";
      description = "Root-only files holding the tokens, which systemd hands the daemon as credentials.";
    };

    api = mkOption {
      type = types.str;
      default = "https://slack.com/api/";
      description = "Slack's web API, ending in /.";
    };
  };

  config = mkIf cfg.enable {
    sops.secrets = mkIf (cfg.sopsFile != null) (
      genAttrs (map secret names) (name: {
        inherit (cfg) sopsFile;
        key = lib.removePrefix "slack/" name;
      })
    );

    systemd.tmpfiles.settings.agent-slack = {
      "/etc/agent-slack".d = {
        mode = "0755";
        user = "root";
        group = "root";
      };
      ${list}.f = {
        mode = "0644";
        user = "root";
        group = "root";
        # a line of its own, so `echo id >> channels` adds the next
        argument = "${cfg.channel}\n";
      };
    };

    # systemd holds the socket, so its owner is set here and clients queue while the daemon restarts
    systemd.sockets.agent-slack = {
      description = "agent-slackd's local API";
      wantedBy = [ "sockets.target" ];
      socketConfig = {
        ListenStream = "/run/agent-slack.sock";
        SocketUser = cfg.user;
        SocketMode = "0600";
        FileDescriptorName = "api";
        Service = "agent-slackd.service";
      };
    };

    systemd.services.agent-slackd = {
      description = "Slack's Socket Mode connection and the agent session's inbox";
      wantedBy = [ "multi-user.target" ];
      requires = [ "agent-slack.socket" ];
      wants = [ "network-online.target" ];
      after = [
        "agent-slack.socket"
        "network-online.target"
        "systemd-tmpfiles-setup.service"
      ];
      startLimitIntervalSec = 0;
      serviceConfig = {
        ExecStart = "${getExe' agent-slack "agent-slackd"} --api ${cfg.api} --state \${STATE_DIRECTORY} --credentials \${CREDENTIALS_DIRECTORY}";
        Sockets = [ "agent-slack.socket" ];
        DynamicUser = true;
        StateDirectory = "agent-slack";
        StateDirectoryMode = "0700";
        LoadCredential = map (name: "${name}:${cfg.tokens.${name}}") names;
        ProtectHome = true;
        RestrictAddressFamilies = [
          "AF_UNIX"
          "AF_INET"
          "AF_INET6"
        ];
        Restart = "always";
        RestartSec = "10s";
      };
    };

    users.users.${cfg.user}.packages = [ agent-slack ];
  };
}
