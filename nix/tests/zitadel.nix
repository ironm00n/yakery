{ pkgs }:
let
  zitadel = import ./zitadel-server.nix { inherit pkgs; };
  inherit (zitadel) host secretsDir;
  addr = "192.168.1.1";
  scope = "openid profile urn:zitadel:iam:org:project:id:zitadel:aud";
in
pkgs.testers.runNixOSTest {
  name = "zitadel-bundle";

  defaults.virtualisation.graphics = false;

  nodes.machine = {
    imports = [ zitadel.node ];
    virtualisation.memorySize = 2048;
    # the classic login, as cx23-2's instance serves it
    services.zitadel.settings.DefaultInstance.Features.LoginV2.Required = false;
  };

  testScript =
    { nodes, ... }:
    let
      dump = nodes.machine.bundles.backup.services.zitadel.dumpCommand;
    in
    zitadel.python
    + ''
      import base64
      import re
      import urllib.parse

      zitadel = machine
      https = "curl -fsS -k --resolve ${host}:443:${addr} https://${host}"
      ready = "curl -fsS http://127.0.0.1:${toString nodes.machine.services.zitadel.settings.Port}/debug/ready"

      machine.wait_for_unit("zitadel.service")
      machine.wait_until_succeeds(ready, timeout=300)
      machine.wait_for_unit("nginx.service")

      with subtest("the sandboxed unit serves through the proxy"):
          discovery = json.loads(machine.succeed(f"{https}/.well-known/openid-configuration"))
          assert discovery["issuer"] == "https://${host}", discovery["issuer"]
          keys = json.loads(machine.succeed(f"{https}/oauth/v2/keys"))["keys"]
          assert len(keys) > 0, keys
          machine.succeed(f"{https}/ui/console/ | grep -q '<cnsl-root'")
          machine.succeed(f"{https}/ui/login/resources/themes/zitadel/css/zitadel.css -o /dev/null")

      with subtest("zitadel reaches postgres only as its own role, despite the secret's stale credentials"):
          roles = machine.succeed(
              "runuser -u postgres -- psql -tAc \"select distinct usename from pg_stat_activity where datname = 'zitadel'\""
          ).split()
          assert roles == ["zitadel"], roles

      with subtest("a client-credentials JWT carries the requested scopes"):
          pat = admin_pat()
          uid = api("GET", "/auth/v1/users/me", pat)["user"]["id"]
          api("PUT", f"/management/v1/users/{uid}/machine", pat, {"name": "probe", "accessTokenType": "ACCESS_TOKEN_TYPE_JWT"})
          secret = api("PUT", f"/management/v1/users/{uid}/secret", pat, {})
          token = json.loads(machine.succeed(
              f"{https}/oauth/v2/token -u '{secret['clientId']}:{secret['clientSecret']}'"
              " -d grant_type=client_credentials --data-urlencode 'scope=${scope}'"
          ))["access_token"]
          payload = token.split(".")[1]
          claims = json.loads(base64.urlsafe_b64decode(payload + "=" * (-len(payload) % 4)))
          assert claims.get("scope") == "${scope}", claims

      # an IdP whose endpoints nothing serves: being sent to it is a redirect to its authorize URL
      idp_authorize = "https://idp.example.test/authorize"
      pat = admin_pat()
      idp = api("POST", "/admin/v1/idps/oauth", pat, {
          "name": "idp", "clientId": "zitadel", "clientSecret": "secret", "authorizationEndpoint": idp_authorize,
          "tokenEndpoint": "https://idp.example.test/token", "userEndpoint": "https://idp.example.test/user",
          "scopes": ["openid"], "idAttribute": "sub", "providerOptions": {"isLinkingAllowed": True},
      })["id"]
      api("POST", "/admin/v1/policies/login/idps", pat, {"idpId": idp})
      project = api("POST", "/management/v1/projects", pat, {"name": "login"})["id"]
      client = api("POST", f"/management/v1/projects/{project}/apps/oidc", pat, {
          "name": "login", "redirectUris": ["http://localhost/cb"], "responseTypes": ["OIDC_RESPONSE_TYPE_CODE"],
          "grantTypes": ["OIDC_GRANT_TYPE_AUTHORIZATION_CODE"], "appType": "OIDC_APP_TYPE_USER_AGENT",
          "authMethodType": "OIDC_AUTH_METHOD_TYPE_NONE", "devMode": True,
      })["clientId"]

      def linked_user(name, **password):
          uid = api("POST", "/v2/users/human", pat, {
              "username": name, "profile": {"givenName": name, "familyName": name},
              "email": {"email": f"{name}@example.test", "isVerified": True},
              "idpLinks": [{"idpId": idp, "userId": f"ext-{name}", "userName": name}], **password,
          })["userId"]
          return api("GET", f"/v2/users/{uid}", pat)["user"]["preferredLoginName"]

      # an authorization request through the classic login, up to the step after the login name or the IdP's button
      def after(login_name=None, idp_button=None):
          jar = "/tmp/login.jar"
          curl = f"curl -sS -k --resolve ${host}:443:${addr} -b {jar} -c {jar} -H 'Referer: https://${host}/ui/login/login'"
          machine.succeed(f"rm -f {jar}")
          query = urllib.parse.urlencode({
              "client_id": client, "redirect_uri": "http://localhost/cb", "response_type": "code", "scope": "openid",
              "code_challenge": "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM", "code_challenge_method": "S256",
          })
          login = machine.succeed(f"{curl} -o /dev/null -w '%{{redirect_url}}' 'https://${host}/oauth/v2/authorize?{query}'")
          page = machine.succeed(f"{curl} '{login}'")

          def field(name):
              found = re.search(f'name="{re.escape(name)}" value="([^"]+)"', page)
              assert found, (name, page[-500:])
              return found.group(1)

          request = field("authRequestID")
          if idp_button:
              step = f"{curl} 'https://${host}/ui/login/login/externalidp?authRequestID={request}&idpConfigID={idp_button}'"
          else:
              form = urllib.parse.urlencode({"gorilla.csrf.Token": field("gorilla.csrf.Token"), "authRequestID": request, "loginName": login_name})
              step = f"{curl} --data '{form}' 'https://${host}/ui/login/loginname'"
          body, status = machine.succeed(f"{step} -w '\\n%{{http_code}} %{{redirect_url}}'").rsplit("\n", 1)
          code, _, location = status.partition(" ")
          return code, location, body

      def sent_to_idp(step):
          code, location, _ = step
          return code == "302" and location.startswith(idp_authorize)

      with_password = linked_user("both", password={"password": "Password1!", "changeRequired": False})
      idp_only = linked_user("idponly")

      with subtest("a user with a password and a linked IdP is asked for the password after the login name"):
          code, location, body = after(login_name=with_password)
          assert code == "200" and 'name="password"' in body, (code, location, body[-500:])

      with subtest("the same user picking the IdP on the login page is sent to it"):
          step = after(idp_button=idp)
          assert sent_to_idp(step), step[:2]

      with subtest("a user with a linked IdP and no password is sent to it after the login name"):
          step = after(login_name=idp_only)
          assert sent_to_idp(step), step[:2]

      with subtest("the backup dump connects over the socket and restores"):
          machine.succeed(
              "${dump} > /tmp/zitadel.dump",
              "runuser -u postgres -- createdb restored",
              "runuser -u postgres -- pg_restore --no-owner --dbname=restored /tmp/zitadel.dump",
          )
          events = int(machine.succeed("runuser -u postgres -- psql restored -tAc 'select count(*) from eventstore.events2'"))
          assert events > 0, events

      with subtest("a missing secret fails the unit instead of starting on defaults"):
          machine.succeed("systemctl stop zitadel", "mv ${secretsDir}/zitadel-settings /root/settings")
          machine.fail("systemctl start zitadel")
          machine.succeed("journalctl -u zitadel -b | grep -i credential")
          machine.succeed("mv /root/settings ${secretsDir}/zitadel-settings", "systemctl start zitadel")
          machine.wait_until_succeeds(ready, timeout=120)

      with subtest("exposure"):
          print(machine.succeed("systemd-analyze security --no-pager zitadel.service | tail -1"))
    '';
}
