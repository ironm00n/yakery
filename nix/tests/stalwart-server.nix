# A Stalwart node on the internal directory, with the recovery admin enabled so a test
# script can create app passwords, and Python helpers for its JMAP API.
{
  pkgs,
  domain ? "example.test",
}:
rec {
  inherit domain;
  recoveryPassword = "recovery";

  account = name: extra: {
    "@type" = "upsert";
    object = "Account";
    matchOn = [
      "name"
      "domainId"
    ];
    value.${name} = {
      "@type" = "User";
      inherit name;
      domainId = "#domain";
      roles."@type" = "User";
      permissions."@type" = "Inherit";
      encryptionAtRest."@type" = "Disabled";
    }
    // extra;
  };

  node = {
    imports = [ ../../modules/stalwart ];
    disabledModules = [ "services/mail/stalwart.nix" ];

    services.stalwart = {
      enable = true;
      package = pkgs.callPackage ../../packages/stalwart { webuiOauthClientId = "test"; };
      recoveryMode = {
        forceEnableUser = true;
        passwordFile = pkgs.writeText "recovery-password" recoveryPassword;
      };
      # before the tests' accounts, which refer to it
      plan.sequence = pkgs.lib.mkBefore [
        {
          "@type" = "upsert";
          object = "Domain";
          matchOn = [ "name" ];
          value.domain = {
            name = domain;
            dkimManagement."@type" = "Manual";
          };
        }
      ];
    };

    networking.firewall.allowedTCPPorts = [ 8080 ];
  };

  # `server` must be the name of the node above
  python = ''
    import base64
    import json
    import shlex

    def basic(user, secret):
        return "Basic " + base64.b64encode(f"{user}:{secret}".encode()).decode()

    # the server reaches itself over loopback, other nodes over the test network
    def curl(node, auth, path, body=None):
        host = "127.0.0.1" if node is server else "server"
        data = f"-H 'Content-Type: application/json' --data {shlex.quote(json.dumps(body))}" if body else ""
        out = node.succeed(
            f"curl -s -w '\\n%{{http_code}}' -H {shlex.quote('Authorization: ' + auth)} {data} http://{host}:8080{path}"
        )
        payload, status = out.rsplit("\n", 1)
        return int(status), payload

    def session(node, auth):
        status, payload = curl(node, auth, "/jmap/session")
        return status, (json.loads(payload) if status == 200 else None)

    def call(auth, account_id, method, args):
        status, payload = curl(server, auth, "/jmap/", {
            "using": ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail", "urn:stalwart:jmap"],
            "methodCalls": [[method, {"accountId": account_id, **args}, "0"]],
        })
        assert status == 200, payload
        name, result, _ = json.loads(payload)["methodResponses"][0]
        assert name == method, result
        return result

    admin = basic("recovery-admin", "${recoveryPassword}")
    admin_id: str
    account_ids: dict[str, str]

    # stalwart-apply-plan links the plan it applied and only then restarts out of recovery mode
    def wait_for_stalwart():
        server.wait_until_succeeds(
            "test -L /var/lib/stalwart/.plan.ndjson"
            " && [ $(systemctl show -P ActiveState stalwart-apply-plan.service) = inactive ]"
        )
        assert server.succeed("systemctl show -P Result stalwart-apply-plan.service").strip() == "success"
        server.wait_for_unit("stalwart.service")
        server.wait_until_succeeds("curl -sf http://127.0.0.1:8080/healthz/ready")
        server.wait_until_succeeds(f"curl -sf -H {shlex.quote('Authorization: ' + admin)} http://127.0.0.1:8080/jmap/session")
        global admin_id, account_ids
        admin_id = session(server, admin)[1]["primaryAccounts"]["urn:stalwart:jmap"]
        query = call(admin, admin_id, "x:Account/query", {})
        accounts = call(admin, admin_id, "x:Account/get", {"ids": query["ids"], "properties": ["name"]})
        account_ids = {a["name"]: a["id"] for a in accounts["list"]}

    def app_password(name):
        created = call(admin, admin_id, "x:AppPassword/set", {
            "accountId": account_ids[name],
            "create": {"p": {
                "description": "test",
                "allowedIps": {"127.0.0.1/32": True, "::1/128": True},
                "permissions": {"@type": "Inherit"},
            }},
        })
        assert "p" in created.get("created", {}), created
        return created["created"]["p"]["secret"]
  '';
}
