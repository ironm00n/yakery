# Stalwart's HTTP rate limit, 1000 requests a minute for an account, spares Owen's hosts, where every session signs in
# as agent@ (through nginx, over NetBird), and still applies to anyone else.
{ pkgs }:
let
  inherit (pkgs) lib;
  stalwart = import ./stalwart-server.nix { inherit pkgs; };
  inherit (stalwart) my-lib;
  inherit (my-lib.ironmoon.mail) loopback;
  owens = lib.concatMap my-lib.netbird.addresses [
    "desktop"
    "fw12"
    "fw13"
  ];
  stranger = "192.0.2.77";
in
pkgs.testers.runNixOSTest {
  name = "stalwart-rate-limit";

  defaults.virtualisation.graphics = false;

  nodes.server = {
    imports = [
      (stalwart.node { users = [ "agent" ]; })
      ../../hosts/hetzner-cx33-1/ban-exempt.nix
    ];
  };

  testScript = stalwart.python + ''
    import collections

    start_all()
    wait_for_stalwart()

    created = call(admin, admin_id, "x:AppPassword/set", {
        "accountId": account_ids["agent"],
        "create": {"p": {
            "description": "burst",
            "allowedIps": {cidr: True for cidr in ${
              builtins.toJSON (my-lib.netbird.overlayCidrs ++ [ "${stranger}/32" ])
            }},
            "permissions": {"@type": "Inherit"},
        }},
    })
    assert "p" in created.get("created", {}), created
    auth = basic("agent@${stalwart.domain}", created["created"]["p"]["secret"])

    # past the limit within its minute, as nginx relays a client at `forwarded`
    def burst(forwarded, n=1100):
        server.succeed(
            f"for i in $(seq {n}); do printf 'url = \"http://${loopback.address}:${toString loopback.port}/jmap/session\"\\noutput = \"/dev/null\"\\n'; done > /tmp/burst.cfg"
        )
        out = server.succeed(
            f"curl -s -w '%{{http_code}}\\n' -H {shlex.quote('Authorization: ' + auth)}"
            f" -H 'X-Forwarded-For: {forwarded}' -K /tmp/burst.cfg"
        )
        return dict(collections.Counter(out.split()))

    with subtest("requests from Owen's hosts are never limited"):
        codes = {forwarded: burst(forwarded) for forwarded in ${builtins.toJSON owens}}
        assert all(c == {"200": 1100} for c in codes.values()), codes

    with subtest("anyone else's are, past 1000 a minute"):
        codes = burst("${stranger}")
        assert codes.get("429", 0) > 0, codes
  '';
}
