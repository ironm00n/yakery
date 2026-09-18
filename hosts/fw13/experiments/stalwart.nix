{
  pkgs,
  lib,
  my-lib,
  config,
  inputs,
  ...
}:
let
  main-domain-id = "ironmoon";
  main-domain = "ironmoon.dev";

  stalwart-secrets =
    my-lib.sops.mkSecrets
      {
        inherit config;
        sopsFile = inputs.secrets.lib.stalwart;
        prefix = "stalwart";
        separator = "-";
        usergroup = "stalwart";
      }
      [
        "recovery_admin_password"
      ];
  get-stalwart-secret = stalwart-secrets.get-path;

  # Stalwart represents arrays as objects keyed by stringified indices.
  # Recursively convert lists to that shape so configs use normal Nix lists.
  listsToIndexedObjects =
    value:
    if builtins.isList value then
      builtins.listToAttrs (
        lib.imap0 (i: v: {
          name = toString i;
          value = listsToIndexedObjects v;
        }) value
      )
    else if builtins.isAttrs value && !lib.isDerivation value then
      builtins.mapAttrs (_: listsToIndexedObjects) value
    else
      value;

  create = object: records: {
    "@type" = "create";
    inherit object;
    value = listsToIndexedObjects records;
  };

  create1 = object: id: body: create object { ${id} = body; };

  update = object: patch: {
    "@type" = "update";
    inherit object;
    value = listsToIndexedObjects patch;
  };

  destroyAll = object: {
    "@type" = "destroy";
    inherit object;
  };

  destroyWhere = object: filter: {
    "@type" = "destroy";
    inherit object;
    value = listsToIndexedObjects filter;
  };

  upsert = object: value: {
    "@type" = "upsert";
    inherit object;
    value = listsToIndexedObjects value;
  };
  upsert1 = object: id: body: upsert object { ${id} = body; };

  upsertOn = object: matchOn: value: {
    "@type" = "upsert";
    inherit object matchOn;
    value = listsToIndexedObjects value;
  };

  upsertOn1 = object: id: matchOn: body: upsertOn object matchOn { ${id} = body; };

  reconcile = object: matchOn: value: {
    "@type" = "reconcile";
    inherit object matchOn;
    value = listsToIndexedObjects value;
  };

  ref = id: "#${id}";

  stalwart-pkg = pkgs.callPackage ../../../packages/stalwart {
    webuiOauthClientId = "373369290327392257";
  };

  toSet = names: lib.genAttrs names (_: true);

  # Built-in role permission sets, re-derived from `perms` so they track the
  # deployed Stalwart version instead of freezing at first boot.
  defaults = import ./stalwart-default-perms.nix {
    inherit lib;
    perms = stalwart-pkg.permissions;
  };

  roleDefs = import ./stalwart-roles.nix {
    perms = stalwart-pkg.permissions;
    inherit defaults;
  };
  rolesOp = reconcile "Role" ["description"] (
    lib.mapAttrs (_: {name, extends ? [], enabled ? [], disabled ? []}: {
      description = name;
      roleIds = toSet (map ref extends);
      enabledPermissions = toSet enabled;
      disabledPermissions = toSet disabled;
    }) roleDefs
  );
in
{
  sops.secrets = stalwart-secrets.secrets;

  disabledModules = [ "services/mail/stalwart.nix" ];
  imports = [../../../modules/stalwart];

  services.stalwart = {
    enable = true;
    package = stalwart-pkg;

    recoveryMode = {
      forceEnable = false;
      forceEnableUser = false;
      port = 9999;
      user = "recovery-admin";
      passwordFile = get-stalwart-secret "recovery_admin_password";
    };

    environmentFile = pkgs.writeText "stalwart-env" ''
      STALWART_PUBLIC_URL="https://stalwart.test"
    '';

    plan = {
      enableDefaultPlan = true;
      sequence = [
        (upsertOn1 "Directory" "zitadel-oidc" ["description"] {
          "@type" = "Oidc";
          description = "ironmoon Zitadel";
          issuerUrl = "https://auth.ironmoon.dev";
          requireAudience = null;
          usernameDomain = main-domain;
          # claimGroups = "groups";
          # HACK until https://github.com/zitadel/zitadel/pull/12180 lands
          requireScopes = [];
        })

        rolesOp
        (update "Authentication" {
          directoryId = ref "zitadel-oidc";
          # TODO: do i want to customize?
          defaultUserRoleIds = toSet [(ref "default-user")];
          defaultGroupRoleIds = toSet [(ref "default-group")];
          defaultAdminRoleIds = toSet [(ref "super-admin")];
        })

        (upsertOn1 "Domain" main-domain-id ["name"] {
          name = "${main-domain}";
          dkimManagement."@type" = "Manual";
        })
        (update "SystemSettings" {
          defaultHostname = "mail.${main-domain}";
          defaultDomainId = ref main-domain-id;
        })
        (upsertOn1 "Account" "root" ["name" "domainId"] {
          "@type" = "User";
          name = "root";
          domainId = ref main-domain-id;
          permissions."@type" = "Inherit";
          roles."@type" = "Admin";
          encryptionAtRest."@type" = "Disabled";
        })

        (upsertOn1 "Account" "postmaster" ["name" "domainId"] {
          "@type" = "Group";
          name = "postmaster";
          domainId = ref main-domain-id;
          permissions."@type" = "Inherit";
          roles."@type" = "Custom";
          roles.roleIds = [];
        })

        # Permissive CORS so the browser app at bulwark.test may call the
        # cross-origin JMAP API on stalwart.test.
        (update "Http" {
          usePermissiveCors = true;
        })
      ];
    };
  };
}
