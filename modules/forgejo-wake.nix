{
  config,
  lib,
  pkgs,
  my-lib,
  ...
}:
let
  cfg = config.services.forgejo-wake;
  forgejo = config.services.forgejo;
  package = pkgs.callPackage ../crates/rust.nix { crate = "forgejo-wake"; };
  # a loopback address of its own, so opening it to Forgejo's webhooks opens nothing else here
  address = "127.0.0.2";
  url = "http://${address}:${toString cfg.port}/";
  stateDir = "/var/lib/forgejo-wake";
  cli = "${lib.getExe forgejo.package} --work-path ${forgejo.stateDir} --config ${forgejo.customDir}/conf/app.ini";
  hooks = "http://${forgejo.settings.server.HTTP_ADDR}:${toString forgejo.settings.server.HTTP_PORT}/api/v1/repos/${cfg.repository}/hooks";
in
{
  options.services.forgejo-wake = {
    enable = lib.mkEnableOption ''
      a mail for each person's comment on {option}`services.forgejo-wake.repository`, to the
      lane its issue's owner label names, so the session watching that lane's topic wakes
    '';
    repository = lib.mkOption {
      type = lib.types.str;
      example = "agents/work";
      description = "The repository, `owner/name`.";
    };
    account = lib.mkOption {
      type = lib.types.str;
      description = ''
        A user administering the repository. The webhook is kept with a token of its, made on
        this host and kept in ${stateDir}/token; to replace it, delete the file and restart
        forgejo-wake-hook. Its own comments wake no one.
      '';
    };
    label = lib.mkOption {
      type = lib.types.str;
      default = "owner/";
      description = "What an owner label starts with; the rest names the lane.";
    };
    routes = lib.mkOption {
      type = lib.types.attrsOf lib.types.str;
      default = { };
      description = "Lanes mailed in another's place.";
    };
    fallback = lib.mkOption {
      type = lib.types.str;
      description = "The lane of an issue no owner label names.";
    };
    to = lib.mkOption {
      type = lib.types.str;
      example = "agent+{lane}@example.org";
      description = "Where a lane's mail goes, `{lane}` standing for the lane.";
    };
    smtp = {
      host = lib.mkOption {
        type = lib.types.str;
        description = "The submission server, reached with STARTTLS.";
      };
      port = lib.mkOption {
        type = lib.types.port;
        default = 587;
      };
      login = lib.mkOption { type = lib.types.str; };
      from = lib.mkOption {
        type = lib.types.str;
        description = "An address the login may send from.";
      };
      passwordFile = lib.mkOption { type = lib.types.path; };
    };
    port = lib.mkOption {
      type = lib.types.port;
      default = 8082;
      description = "The port on ${address} Forgejo posts to.";
    };
  };

  config = lib.mkIf cfg.enable {
    services.forgejo.settings.webhook.ALLOWED_HOST_LIST = "external,${address}/32";

    systemd.services.forgejo-wake-hook = {
      description = "The webhook on ${cfg.repository} that mails each comment's lane";
      after = [ "forgejo.service" ];
      requires = [ "forgejo.service" ];
      wantedBy = [ "multi-user.target" ];
      path = [
        pkgs.curl
        pkgs.jq
      ];
      serviceConfig = my-lib.systemd.hardening.network // {
        Type = "oneshot";
        RemainAfterExit = true;
        User = forgejo.user;
        Group = forgejo.group;
        StateDirectory = baseNameOf stateDir;
        StateDirectoryMode = "0700";
        UMask = "0077";
      };
      script = ''
        set -euo pipefail
        cd ${stateDir}
        if [ ! -s secret ]; then
          head -c 32 /dev/urandom | base64 -w0 > secret.new
          mv secret.new secret
        fi
        if [ ! -s token ]; then
          ${cli} admin user generate-access-token --username ${cfg.account} \
            --token-name "forgejo-wake-$(date +%s)" --scopes write:repository --raw > token.new
          mv token.new token
        fi
        call() { curl -fsS -H "Authorization: token $(cat token)" -H 'Content-Type: application/json' "$@"; }

        want=$(jq -n --arg url ${url} --rawfile secret secret \
          '{type: "forgejo", active: true, events: ["issue_comment"], config: {url: $url, content_type: "json", secret: $secret}}')
        ours=$(call ${hooks} | jq --arg url ${url} '[.[] | select(.config.url == $url) | .id]')
        for id in $(jq '.[1:][]' <<<"$ours"); do
          call -X DELETE ${hooks}/"$id"
        done
        if [ "$(jq length <<<"$ours")" = 0 ]; then
          call -X POST --data "$want" ${hooks} >/dev/null
        else
          call -X PATCH --data "$(jq 'del(.type)' <<<"$want")" ${hooks}/"$(jq '.[0]' <<<"$ours")" >/dev/null
        fi
      '';
    };

    systemd.sockets.forgejo-wake = {
      wantedBy = [ "sockets.target" ];
      listenStreams = [ "${address}:${toString cfg.port}" ];
    };

    systemd.services.forgejo-wake = {
      description = "Mails the lane of each person's comment on ${cfg.repository}";
      wants = [ "forgejo-wake-hook.service" ];
      after = [ "forgejo-wake-hook.service" ];
      serviceConfig = my-lib.systemd.hardening.network // {
        ExecStart = lib.concatStringsSep " " (
          [
            (lib.getExe package)
            "--secret %d/secret"
            "--password %d/password"
          ]
          ++ lib.mapAttrsToList (flag: value: "--${flag} ${lib.escapeShellArg value}") {
            inherit (cfg.smtp) host login from;
            inherit (cfg) to label fallback;
            port = toString cfg.smtp.port;
            helo = config.networking.hostName;
            quiet = cfg.account;
          }
          ++ lib.mapAttrsToList (
            lane: instead: "--route ${lib.escapeShellArg "${lane}=${instead}"}"
          ) cfg.routes
        );
        LoadCredential = [
          "secret:${stateDir}/secret"
          "password:${cfg.smtp.passwordFile}"
        ];
        DynamicUser = true;
        Restart = "on-failure";
      };
    };
  };
}
