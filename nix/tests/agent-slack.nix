# agent-slackd under the bundle's own units against a fake web API: systemd holds the socket for one user, the
# tokens reach the daemon as credentials that user cannot read, and send and read go through the daemon.
{ pkgs, inputs }:
let
  agent-slack = pkgs.callPackage ../../crates/agent-slack/package.nix { };
  fake = pkgs.writers.writePython3 "fake-slack" { } ''
    import json
    from http.server import BaseHTTPRequestHandler, HTTPServer
    from urllib.parse import parse_qsl

    ANSWERS = {
        "auth.test": {"user_id": "U0BOT"},
        "apps.connections.open": None,
        "chat.postMessage": {"ts": "1800000000.000001"},
        "conversations.replies": {"messages": [
            {"type": "message", "user": "U0PERSON", "text": "first",
             "ts": "1800000000.000001"},
        ], "has_more": False},
    }


    class Fake(BaseHTTPRequestHandler):
        def do_POST(self):
            method = self.path.removeprefix("/api/")
            length = int(self.headers["Content-Length"] or 0)
            form = dict(parse_qsl(self.rfile.read(length).decode()))
            form["method"] = method
            form["authorization"] = self.headers["Authorization"]
            with open("/tmp/calls", "a") as calls:
                calls.write(json.dumps(form) + "\n")
            answer = ANSWERS.get(method)
            body = {"ok": False, "error": "no_socket_here"}
            if answer is not None:
                body = {"ok": True, **answer}
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(json.dumps(body).encode())


    HTTPServer(("127.0.0.1", 8080), Fake).serve_forever()
  '';
in
pkgs.testers.runNixOSTest {
  name = "agent-slack";

  nodes.vm = {
    imports = [
      ../../bundles/agent-slack.nix
      inputs.sops-nix.nixosModules.sops
    ];
    _module.args = { inherit inputs; };
    bundles.agent-slack = {
      enable = true;
      user = "owner";
      sopsFile = null;
      tokens = {
        bot_token = "/etc/slack-test/bot_token";
        app_token = "/etc/slack-test/app_token";
      };
      api = "http://127.0.0.1:8080/api/";
    };
    environment.etc = {
      "slack-test/bot_token" = {
        text = "xoxb-vm";
        mode = "0400";
      };
      "slack-test/app_token" = {
        text = "xapp-vm";
        mode = "0400";
      };
    };
    systemd.services.fake-slack = {
      wantedBy = [ "multi-user.target" ];
      before = [ "agent-slackd.service" ];
      serviceConfig.ExecStart = fake;
    };
    users.users = {
      owner.isNormalUser = true;
      stranger.isNormalUser = true;
    };
  };

  testScript = ''
    import json

    vm.wait_for_unit("agent-slackd.service")
    vm.wait_until_succeeds("journalctl -u agent-slackd -o cat | grep -q no_socket_here")
    assert vm.succeed("stat -c '%U %a' /run/agent-slack.sock").strip() == "owner 600"
    assert vm.succeed("stat -c '%U %a' /etc/agent-slack/channels").strip() == "root 644"
    assert vm.succeed("cat /etc/agent-slack/channels") == "C0CCD4YGE2Y\n"
    vm.fail("su -l owner -c 'cat /run/credentials/agent-slackd.service/bot_token'")
    vm.fail("su stranger -c '${agent-slack}/bin/agent-slack read 1800000000.000001'")

    ts = vm.succeed("echo hello | su -l owner -c 'agent-slack send'").strip()
    assert ts == "1800000000.000001", ts
    posts = [c for c in map(json.loads, vm.succeed("cat /tmp/calls").splitlines())
             if c["method"] == "chat.postMessage"]
    assert posts == [{"method": "chat.postMessage", "authorization": "Bearer xoxb-vm",
                      "channel": "C0CCD4YGE2Y", "markdown_text": "hello\n"}], posts
    vm.fail("echo x | su -l owner -c 'agent-slack send --to C0NOTLISTED'")
    read = vm.succeed("su -l owner -c 'agent-slack read 1800000000.000001'")
    assert read == "ts 1800000000.000001 from U0PERSON\nfirst\n\n", read

    vm.succeed("echo C0SECOND >> /etc/agent-slack/channels")
    vm.succeed("echo y | su -l owner -c 'agent-slack send --to C0SECOND --thread 1800000000.000001'")
    vm.succeed("grep -q '\"channel\": \"C0SECOND\"' /tmp/calls")

    vm.succeed("systemctl restart agent-slackd")
    vm.wait_for_unit("agent-slackd.service")
    vm.succeed("test -d /var/lib/private/agent-slack")
    assert vm.succeed("stat -c %a /var/lib/private/agent-slack").strip() == "700"
  '';
}
