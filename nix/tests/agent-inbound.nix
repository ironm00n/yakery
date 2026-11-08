# cx33-1's Stalwart on port 25: a stranger's mail reaches agent@ and its topics, while an agent on the agents'
# subdomain still takes mail only from its correspondents.
{ pkgs }:
let
  inherit (pkgs) lib;
  stalwart = import ./stalwart-server.nix { inherit pkgs; };
  inherit (stalwart) address;
  inherit (stalwart.my-lib.stalwart) upsertOn ref;
  inherit (stalwart.my-lib.ironmoon.mail) agents;
  stranger = "stranger@example.org";
in
pkgs.testers.runNixOSTest {
  name = "agent-inbound";

  defaults.virtualisation.graphics = false;

  nodes.server = {
    imports = [ (stalwart.node { users = [ agents.sharedLocalPart ]; }) ];
    bundles.stalwart.receiving.enable = true;
    # a resolver that refuses at once, so looking up the stranger's SPF, DKIM and DMARC waits out no timeout
    networking.nameservers = [ "127.0.0.1" ];
    # before the node's users, whose account `agent` would shadow the domain's plan reference
    services.stalwart.plan.sequence = lib.mkOrder 1250 [
      (upsertOn "Account" [ "name" "domainId" ] {
        worker = {
          "@type" = "User";
          name = "worker";
          domainId = ref "agent";
          roles."@type" = "User";
          permissions."@type" = "Inherit";
          encryptionAtRest."@type" = "Disabled";
        };
      })
    ];
    environment.systemPackages = [ pkgs.swaks ];
  };

  testScript =
    { nodes, ... }:
    stalwart.python
    + ''
      start_all()
      wait_for_stalwart()

      def mail(to):
          return server.execute(f"swaks -s ${nodes.server.networking.primaryIPAddress} -p 25 --ehlo example.org -f ${stranger} -t {to} 2>&1")

      with subtest("a stranger's mail reaches agent@ and a topic of it"):
          sent = {to: mail(to) for to in ["${address agents.sharedLocalPart}", "${address "${agents.sharedLocalPart}+x"}"]}
          assert all(status == 0 for status, _ in sent.values()), sent
          agent = sign_in(["${agents.sharedLocalPart}"])["${agents.sharedLocalPart}"]
          def both(_):
              found = call(agent, account_ids["${agents.sharedLocalPart}"], "Email/query", {"filter": {"from": "${stranger}"}})
              return len(found["ids"]) == 2
          retry(both, timeout_seconds=60)

      with subtest("an agent on the subdomain still refuses it"):
          status, out = mail("worker@${agents.domain}")
          assert status != 0 and "550 5.7.1 agents take mail only from their correspondents" in out, out
    '';
}
