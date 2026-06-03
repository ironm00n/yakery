{
  config,
  lib,
  my-lib,
  ...
}:
let
  inherit (lib)
    mkEnableOption
    mkOption
    mkIf
    types
    mapAttrs
    optionalAttrs
    ;
  cfg = config.bundles.reverse-proxy;

  anyTls = lib.any (h: h.tls) (lib.attrValues cfg.hosts);

  bracketV6 = a: if lib.hasInfix ":" a && !lib.hasPrefix "[" a then "[${a}]" else a;

  san = host: lib.replaceStrings [ "." "-" "*" ] [ "_" "_" "_" ] host;
  geoVar = host: "rp_backend_${san host}";
  nginxVar = host: "$" + geoVar host;

  anubisPortBase = 8920;
  anubisPorts = lib.listToAttrs (
    lib.imap0 (i: h: lib.nameValuePair h (anubisPortBase + i)) (
      lib.sort (a: b: a < b) (lib.attrNames (lib.filterAttrs (_: o: o.anubis) cfg.hosts))
    )
  );

  defaultBackend =
    host: opts:
    if opts.anubis then "127.0.0.1:${toString anubisPorts.${host}}" else "127.0.0.1:${toString opts.port}";

  # Source ranges that should skip the vhost's Anubis filter and hit the backend directly.
  overrides =
    opts:
    lib.optional (opts.anubis && cfg.trustedSources != [ ]) {
      sources = cfg.trustedSources;
      port = opts.port;
    };

  mkGeo =
    host: opts:
    let
      entries = lib.concatLists (
        map (b: map (cidr: "    ${cidr} 127.0.0.1:${toString b.port};") b.sources) (overrides opts)
      );
    in
    lib.concatStringsSep "\n" (
      [
        "geo ${nginxVar host} {"
        "    default ${defaultBackend host opts};"
      ]
      ++ entries
      ++ [ "}" ]
    );

  geoBlocks = lib.concatStringsSep "\n\n" (
    lib.mapAttrsToList mkGeo (lib.filterAttrs (_: o: o.port != null && overrides o != [ ]) cfg.hosts)
  );

  hostOpts.options = {
    port = mkOption {
      type = types.nullOr types.port;
      default = null;
      description = ''
        Backend port on 127.0.0.1 to proxy to.
        When null, the vhost is created with `enableACME` and `forceSSL` only.
      '';
    };
    anubis = mkOption {
      type = types.bool;
      default = false;
      description = "Front this vhost with an Anubis proof-of-work filter; clients in `trustedSources` bypass it. Requires `port`.";
    };
    tls = mkOption {
      type = types.bool;
      default = true;
      description = "Issue an ACME cert and force SSL. When false, serve plain HTTP only.";
    };
    websockets = mkOption {
      type = types.bool;
      default = false;
      description = "Enable `proxyWebsockets` on the root location.";
    };
    aliases = mkOption {
      type = types.listOf types.str;
      default = [ ];
      description = "`serverAliases` for this vhost.";
    };
    listenAddresses = mkOption {
      type = types.listOf types.str;
      default = [ ];
      description = "Addresses to bind this vhost to (empty = nginx defaults; IPv6 auto-bracketed).";
    };
    extraLocationConfig = mkOption {
      type = types.lines;
      default = "";
      description = "Extra config appended to the root location block.";
    };
    extraVhostConfig = mkOption {
      type = types.lines;
      default = "";
      description = "Extra config appended at the vhost level.";
    };
    hsts = mkOption {
      type = types.submodule {
        options = {
          enable = mkOption {
            type = types.bool;
            default = false;
            description = ''
              Add a `Strict-Transport-Security` header.
            '';
          };
          max-age = mkOption {
            type = types.ints.positive;
            default = 2 * 365 * 24 * 60 * 60; # 2 years; HSTS preload minimum is 1 year
            description = "Value of the `max-age` directive, in seconds.";
          };
          include-subdomains = mkOption {
            type = types.bool;
            default = false;
            description = "Include the `includeSubDomains` directive.";
          };
          preload = mkOption {
            type = types.bool;
            default = false;
            description = "Include the `preload` directive (required for HSTS preload list submission).";
          };
        };
      };
      default = { };
      description = "HTTP Strict Transport Security configuration for this vhost.";
    };
  };

  mkHstsHeader =
    hsts:
    let
      directives = [
        "max-age=${toString hsts.max-age}"
      ]
      ++ lib.optional hsts.include-subdomains "includeSubDomains"
      ++ lib.optional hsts.preload "preload";
    in
    ''add_header Strict-Transport-Security "${lib.concatStringsSep "; " directives}" always;'';

  mkVhost =
    name: opts:
    let
      vhostConfig = lib.concatStringsSep "\n" (
        lib.optional opts.hsts.enable (mkHstsHeader opts.hsts)
        ++ lib.optional (opts.extraVhostConfig != "") opts.extraVhostConfig
      );
      backend =
        if overrides opts == [ ] then
          "http://${defaultBackend name opts}"
        else
          "http://${nginxVar name}$request_uri";
    in
    optionalAttrs opts.tls {
      enableACME = true;
      forceSSL = true;
    }
    // optionalAttrs (opts.aliases != [ ]) { serverAliases = opts.aliases; }
    // optionalAttrs (opts.listenAddresses != [ ]) {
      listenAddresses = map bracketV6 opts.listenAddresses;
    }
    // optionalAttrs (vhostConfig != "") { extraConfig = vhostConfig; }
    // optionalAttrs (opts.port != null) {
      locations."/" = {
        proxyPass = backend;
      }
      // optionalAttrs opts.websockets { proxyWebsockets = true; }
      // optionalAttrs (opts.extraLocationConfig != "") { extraConfig = opts.extraLocationConfig; };
    };
in
{
  options.bundles.reverse-proxy = {
    enable = mkEnableOption "nginx-based reverse proxy with ACME";

    acme-email = mkOption {
      type = types.nullOr types.str;
      default = null;
      description = "Email for Let's Encrypt registration. Required when the bundle is enabled.";
    };

    trustedSources = mkOption {
      type = types.listOf types.str;
      default = my-lib.netbird.overlayCidrs;
      defaultText = lib.literalExpression "my-lib.netbird.overlayCidrs";
      description = "Source CIDRs that bypass per-vhost Anubis filtering (default: the Netbird overlay).";
    };

    reject-unknown = mkOption {
      type = types.bool;
      default = true;
      description = ''
        Add a `_default_` vhost that rejects TLS connections to unknown hostnames
        (returns HTTP 418 I'm a teapot).
      '';
    };

    openFirewall = mkOption {
      type = types.bool;
      default = false;
      description = ''
        Open TCP 80 (ACME HTTP-01) and 443 (HTTPS) in the firewall. For anything
        beyond that, manage `networking.firewall.allowedTCPPorts` directly.
      '';
    };

    hosts = mkOption {
      type = types.attrsOf (types.submodule hostOpts);
      default = { };
      description = "Reverse proxy vhost definitions, keyed by hostname.";
    };
  };

  config = mkIf cfg.enable {
    assertions = [
      {
        assertion = !anyTls || cfg.acme-email != null;
        message = "bundles.reverse-proxy.acme-email must be set when any vhost uses TLS.";
      }
    ]
    ++ lib.mapAttrsToList (host: opts: {
      assertion = !opts.anubis || opts.port != null;
      message = ''bundles.reverse-proxy.hosts."${host}": `anubis` requires `port` to be set.'';
    }) cfg.hosts;

    services.nginx = {
      enable = true;
      recommendedTlsSettings = lib.mkDefault true;
      recommendedProxySettings = lib.mkDefault true;
      recommendedOptimisation = lib.mkDefault true;
      recommendedGzipSettings = lib.mkDefault true;

      appendHttpConfig = geoBlocks;

      virtualHosts =
        mapAttrs mkVhost cfg.hosts
        // optionalAttrs cfg.reject-unknown {
          "_default_" = {
            default = true;
            rejectSSL = true;
            locations."/".return = "444";
          };
        };
    };

    services.anubis.instances = lib.mapAttrs' (
      host: opts:
      lib.nameValuePair (san host) {
        settings = {
          TARGET = "http://127.0.0.1:${toString opts.port}";
          BIND = "127.0.0.1:${toString anubisPorts.${host}}";
          BIND_NETWORK = "tcp";
          OG_PASSTHROUGH = true;
        };
        policy.extraBots = [
          { import = "(data)/meta/messengers-preview.yaml"; }
          {
            name = "link-preview-extra";
            user_agent_regex = "Discordbot|Slackbot|Twitterbot|facebookexternalhit|WhatsApp|LinkedInBot|Mastodon";
            action = "ALLOW";
          }
        ];
      }
    ) (lib.filterAttrs (_: o: o.anubis) cfg.hosts);

    security.acme = {
      acceptTerms = true;
      defaults.email = cfg.acme-email;
    };

    networking.firewall.allowedTCPPorts = lib.optionals cfg.openFirewall [
      80
      443
    ];
  };
}
