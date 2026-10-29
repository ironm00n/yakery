{ pkgs }:
let
  inherit (pkgs) lib;
  stalwart = import ./stalwart-server.nix { inherit pkgs; };
  inherit (stalwart) domain;
  inherit (stalwart.my-lib.stalwart) upsertOn1 update;
  signer = pkgs.callPackage ../../crates/rust.nix { crate = "submission-signer"; };
  seed = "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc";
  sink = {
    port = 2525;
    dir = "/var/lib/sink";
  };
in
pkgs.testers.runNixOSTest {
  name = "submission-signer";

  defaults.virtualisation.graphics = false;

  nodes.server = {
    imports = [
      (stalwart.node {
        users = [
          "owen"
          "agent"
        ];
      })
    ];
    bundles.stalwart.signSubmissions = true;
    sops.secrets.stalwart-submission_seed.text = seed;
    services.stalwart.plan.sequence = lib.mkAfter [
      (upsertOn1 "MtaRoute" "sink" [ "name" ] {
        "@type" = "Relay";
        name = "sink";
        address = "127.0.0.1";
        inherit (sink) port;
        protocol = "smtp";
      })
      (update "MtaOutboundStrategy" {
        route = {
          match = [
            {
              "if" = "is_local_domain(rcpt_domain)";
              "then" = "'local'";
            }
          ];
          "else" = "'sink'";
        };
      })
    ];
    systemd.services.sink = {
      wantedBy = [ "multi-user.target" ];
      serviceConfig.StateDirectory = baseNameOf sink.dir;
      script = "${lib.getExe' pkgs.python3Packages.aiosmtpd "aiosmtpd"} -n -l 127.0.0.1:${toString sink.port} -c aiosmtpd.handlers.Mailbox ${sink.dir}/mail";
    };
    environment.systemPackages = [ signer ];
  };

  testScript = stalwart.python + ''
    import re

    USING = ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail", "urn:ietf:params:jmap:submission"]

    def jmap(auth, method, args):
        status, payload = curl(server, auth, "/jmap/", {"using": USING, "methodCalls": [[method, args, "0"]]})
        assert status == 200, payload
        name, result, _ = json.loads(payload)["methodResponses"][0]
        assert name == method, result
        return result

    # each message travels as base64, so its bytes reach the check as Stalwart holds them
    def raw(name, blob, path):
        auth = shlex.quote("Authorization: " + logins[name])
        server.succeed(f"curl -sf -o {path} -H {auth} {origin(server)}/jmap/download/{account_ids[name]}/{blob}/m.eml")
        return base64.b64decode(server.succeed(f"base64 -w0 {path}"))

    def check(path):
        return server.execute(f"submission-signer check --public $(submission-signer public --seed ${pkgs.writeText "seed" seed}) < {path} 2>&1")

    def mailbox(auth, account_id, role):
        boxes = jmap(auth, "Mailbox/get", {"accountId": account_id})["list"]
        return next(b["id"] for b in boxes if b.get("role") == role)

    def identity(auth, account_id, email):
        ids = [i["id"] for i in jmap(auth, "Identity/get", {"accountId": account_id})["list"] if i["email"] == email]
        if ids:
            return ids[0]
        created = jmap(auth, "Identity/set", {"accountId": account_id, "create": {"i": {"name": "test", "email": email}}})
        return created["created"]["i"]["id"]

    def send(name, sender_from, to, subject, extra=None):
        auth, account_id = logins[name], account_ids[name]
        created = jmap(auth, "Email/set", {"accountId": account_id, "create": {"m": {
            "mailboxIds": {mailbox(auth, account_id, "drafts"): True},
            "from": [{"email": sender_from}], "to": [{"email": to}], "subject": subject,
            "textBody": [{"partId": "1", "type": "text/plain"}], "bodyValues": {"1": {"value": "yes, deploy it"}},
            **(extra or {}),
        }}})["created"]["m"]
        submitted = jmap(auth, "EmailSubmission/set", {"accountId": account_id, "create": {"s": {
            "emailId": created["id"], "identityId": identity(auth, account_id, f"{name}@${domain}"),
        }}})
        return created, submitted

    def delivered(name, subject, path):
        auth, account_id = logins[name], account_ids[name]
        ids = []
        def found(_):
            ids[:] = jmap(auth, "Email/query", {"accountId": account_id, "filter": {"subject": subject}})["ids"]
            return bool(ids)
        retry(found, timeout_seconds=60)
        blob = jmap(auth, "Email/get", {"accountId": account_id, "ids": ids[:1], "properties": ["blobId"]})["list"][0]["blobId"]
        return raw(name, blob, path)

    start_all()
    wait_for_stalwart()
    logins = {n: basic(f"{n}@${domain}", app_password(n)) for n in ["owen", "agent"]}

    with subtest("mail an account sends to a local domain arrives signed over the bytes it sent"):
        draft, submitted = send("owen", "owen@${domain}", "agent+deploy@${domain}", "signed")
        assert "s" in submitted.get("created", {}), submitted
        message = delivered("agent", "signed", "/tmp/signed.eml")
        status, stamp = check("/tmp/signed.eml")
        print(stamp)
        assert status == 0, stamp
        assert re.fullmatch(r"v=1; account=\S+; sender=owen@${domain}; t=\d+\n", stamp), stamp
        header = re.search(rb"^Submission-Signature: .*\r\n", message, re.M)
        assert header, message
        sent = raw("owen", draft["blobId"], "/tmp/sent.eml")
        assert message[header.end():] == sent, "the bytes below the header are not the bytes sent"

    with subtest("a subaddress of the sender's own address is its own"):
        send("owen", "owen+x@${domain}", "agent@${domain}", "subaddress")
        delivered("agent", "subaddress", "/tmp/subaddress.eml")
        assert check("/tmp/subaddress.eml")[0] == 0

    with subtest("a From that isn't the sender's own address is refused and nothing arrives"):
        _, submitted = send("owen", "agent@${domain}", "agent@${domain}", "forged")
        print(submitted)
        assert "s" in submitted.get("notCreated", {}), submitted
        assert "From is not owen@${domain}" in json.dumps(submitted), submitted

    with subtest("a submission carrying a signature of its own is refused"):
        _, submitted = send("owen", "owen@${domain}", "agent@${domain}", "planted",
                            {"header:Submission-Signature:asText": "v=1; account=owen; sender=owen@${domain}; t=1; sig=x"})
        assert "s" in submitted.get("notCreated", {}), submitted

    with subtest("mail for other domains leaves without the header"):
        _, submitted = send("owen", "owen@${domain}", "someone@remote.test", "outside")
        assert "s" in submitted.get("created", {}), submitted
        server.wait_until_succeeds("grep -rl 'Subject: outside' ${sink.dir}/mail", timeout=60)
        assert server.execute("grep -rli '^Submission-Signature' ${sink.dir}/mail")[0] != 0

    with subtest("a copy changed below the header does not verify"):
        copied = delivered("agent", "signed", "/tmp/signed.eml").replace(b"yes, deploy it", b"yes, deploy all")
        server.succeed(f"echo {base64.b64encode(copied).decode()} | base64 -d > /tmp/copied.eml")
        status, why = check("/tmp/copied.eml")
        assert status != 0 and "the signature does not verify" in why, why

    with subtest("the hook answers no one without the token"):
        status = server.succeed("curl -s -o /dev/null -w '%{http_code}' -X POST -H 'Authorization: Bearer nope' --data '{}' http://127.0.0.1:8081/")
        assert status == "401", status
  '';
}
