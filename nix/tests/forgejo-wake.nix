# oracle-a1-flex-1's Forgejo and its comment route, mailing through cx33-1's Stalwart: a person's comment on an
# issue mails the lane its owner label names, From code@ and threaded under Forgejo's own mail for the issue; an
# issue without one mails dispatch, as does one Owen owns; the agent's own comment and an unsigned request mail no
# one; the webhook stays one however often it is kept. The lane's topic mail is read with JMAP on the agent
# account, where agent-mail's wait would find it.
{ pkgs }:
let
  inherit (pkgs) lib;
  stalwart = import ./stalwart-server.nix { inherit pkgs; };
  inherit (stalwart) domain address;
  seed = "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc";
  login = address "forgejo";
  passwordFile = "/run/forgejo-smtp-password";
in
pkgs.testers.runNixOSTest {
  name = "forgejo-wake";

  defaults.virtualisation.graphics = false;

  nodes.server =
    { nodes, ... }:
    {
      imports = [
        (stalwart.node {
          users = [ "agent" ];
          ips.oracle-a1-flex-1.ipv4.address = nodes.forgejo.networking.primaryIPAddress;
        })
        ../../hosts/hetzner-cx33-1/program-accounts.nix
      ];
      bundles.stalwart = {
        submission.enable = true;
        signSubmissions = true;
      };
      sops.secrets.stalwart-submission_seed.text = seed;
    };

  nodes.forgejo =
    { lib, ... }:
    {
      imports = [
        stalwart.client
        stalwart.secretFiles
        ../../hosts/oracle-a1-flex-1/forgejo.nix
        ../../hosts/oracle-a1-flex-1/postgres.nix
        ../../bundles/reverse-proxy.nix
      ];
      _module.args = {
        inherit (stalwart) my-lib;
        inputs.secrets.lib.forgejo = null;
      };
      virtualisation.memorySize = 2048;

      # the app password exists only once the server has applied its plan, and the hook's user once made
      sops.secrets.forgejo-smtp_password.path = passwordFile;
      systemd.services.forgejo.wantedBy = lib.mkForce [ ];
      systemd.services.forgejo-wake-hook.wantedBy = lib.mkForce [ ];
      systemd.services.forgejo-zitadel-oauth.enable = false;
      bundles.reverse-proxy.enable = lib.mkForce false;
    };

  testScript =
    { nodes, ... }:
    let
      inherit (nodes.forgejo.services.forgejo) package stateDir customDir;
      inherit (nodes.forgejo.services.forgejo-wake) port;
      appPassword = nodes.server.services.stalwart.appPasswords.${login}.file;
      issues = "https://code.ironmoon.dev/agents/work/issues";
    in
    stalwart.python
    + ''
      def cli(args):
          return forgejo.succeed(
              "runuser -u forgejo -- ${lib.getExe package} --work-path ${stateDir}"
              f" --config ${customDir}/conf/app.ini {args}"
          )

      def api(token, method, path, body=None):
          data = f" --data {shlex.quote(json.dumps(body))}" if body is not None else ""
          out = forgejo.succeed(
              f"curl -sf -X {method} -H {shlex.quote('Authorization: token ' + token)}"
              f" -H 'Content-Type: application/json'{data} http://127.0.0.1:3000/api/v1{path}"
          )
          return json.loads(out) if out.strip() else None

      start_all()
      wait_for_stalwart()
      password = server.succeed("cat ${appPassword}").strip()
      forgejo.succeed(f"(umask 077 && echo {shlex.quote(password)} > ${passwordFile})")

      # under TCG initdb can outlast postgresql's start timeout, and its restart finishes the setup
      forgejo.wait_until_succeeds("systemctl is-active postgresql-setup.service", timeout=900)
      forgejo.systemctl("start forgejo.service")
      forgejo.wait_for_unit("forgejo.service")
      forgejo.wait_for_open_port(3000)
      cli("admin user create --username owen --email owen@${domain} --admin --random-password --must-change-password=false")
      cli("admin user create --username agent --email agent@${domain} --random-password --must-change-password=false")
      owen = cli("admin user generate-access-token --username owen --scopes all --raw").strip()
      agent = cli("admin user generate-access-token --username agent --token-name test --scopes all --raw").strip()
      api(owen, "POST", "/orgs", {"username": "agents"})
      api(owen, "POST", "/orgs/agents/repos", {"name": "work"})
      api(owen, "PUT", "/repos/agents/work/collaborators/agent", {"permission": "admin"})
      label = {name: api(owen, "POST", "/repos/agents/work/labels", {"name": name, "color": "#e11d21"})["id"]
               for name in ["owner/protocols", "owner/owen", "priority/high"]}
      number = {}
      for title, labels in [("deploy it", ["owner/protocols", "priority/high"]), ("unowned", []), ("his", ["owner/owen"])]:
          number[title] = api(owen, "POST", "/repos/agents/work/issues",
                              {"title": title, "labels": [label[l] for l in labels]})["number"]

      with subtest("the webhook stays one however often it is kept"):
          for _ in range(2):
              forgejo.succeed("systemctl restart forgejo-wake-hook.service")
          hooks = api(owen, "GET", "/repos/agents/work/hooks")
          assert [(h["config"]["url"], h["events"]) for h in hooks] == [("http://127.0.0.2:${toString port}/", ["issue_comment"])], hooks

      agent_jmap = sign_in(["agent"])["agent"]

      def mails():
          ids = call(agent_jmap, account_ids["agent"], "Email/query", {})["ids"]
          got = call(agent_jmap, account_ids["agent"], "Email/get", {
              "ids": ids,
              "properties": ["from", "to", "subject", "inReplyTo", "textBody", "bodyValues"],
              "fetchTextBodyValues": True,
          })["list"]
          return [m for m in got if m["to"][0]["email"].startswith("agent+")]

      def text(mail):
          return "".join(mail["bodyValues"][part["partId"]]["value"] for part in mail["textBody"])

      def to_lanes(count):
          found = []
          def arrived(_):
              found[:] = mails()
              return len(found) >= count
          retry(arrived, timeout_seconds=120)
          return found

      with subtest("a person's comment on an issue a lane owns mails that lane, From code@ under the issue's thread"):
          api(owen, "POST", f"/repos/agents/work/issues/{number['deploy it']}/comments", {"body": "@agent can you look"})
          [mail] = to_lanes(1)
          assert [a["email"] for a in mail["to"]] == ["agent+protocols@${domain}"], mail
          assert [a["email"] for a in mail["from"]] == ["${address "code"}"], mail
          assert mail["subject"] == f"Re: [agents/work] deploy it (#{number['deploy it']})", mail
          assert mail["inReplyTo"] == [f"agents/work/issues/{number['deploy it']}@code.ironmoon.dev"], mail
          body = text(mail)
          assert "owen commented on agents/work" in body and "@agent can you look" in body, body
          assert f"${issues}/{number['deploy it']}#issuecomment-" in body, body

      with subtest("the agent's own comment mails no one, and lands on the issue"):
          api(agent, "POST", f"/repos/agents/work/issues/{number['deploy it']}/comments", {"body": "on it"})
          forgejo.wait_until_succeeds(
              f"journalctl -u forgejo-wake.service -o cat | grep -qF 'agents/work#{number['deploy it']}: a comment by agent wakes no one'",
              timeout=60,
          )
          comments = api(owen, "GET", f"/repos/agents/work/issues/{number['deploy it']}/comments")
          assert [(c["user"]["login"], c["body"]) for c in comments] == [("owen", "@agent can you look"), ("agent", "on it")], comments
          assert len(mails()) == 1, mails()

      with subtest("an issue without an owner label mails dispatch, and so does one Owen owns"):
          for title in ["unowned", "his"]:
              api(owen, "POST", f"/repos/agents/work/issues/{number[title]}/comments", {"body": f"about {title}"})
          dispatched = [m for m in to_lanes(3) if m["to"][0]["email"] == "agent+dispatch@${domain}"]
          assert sorted(m["subject"] for m in dispatched) == sorted(
              f"Re: [agents/work] {t} (#{number[t]})" for t in ["unowned", "his"]
          ), dispatched

      with subtest("a request the webhook's secret didn't sign is refused"):
          status = forgejo.succeed(
              "curl -s -o /dev/null -w '%{http_code}' -X POST -H 'X-Forgejo-Event: issue_comment'"
              " -H 'X-Forgejo-Signature: 00' --data '{}' http://127.0.0.2:${toString port}/"
          )
          assert status == "401", status
          assert len(mails()) == 3, mails()
    '';
}
