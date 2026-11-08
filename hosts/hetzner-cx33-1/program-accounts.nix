{
  inputs,
  lib,
  my-lib,
  ...
}:
let
  # A host signs in from its public IPv4, from its /64 when it reaches mail.ironmoon.dev over IPv6,
  # and from its overlay addresses when it is a NetBird peer, whose DNS sends it there
  from =
    host:
    let
      ip = inputs.secrets.data.ips.${host};
    in
    [ ip.ipv4.address ]
    ++ lib.optional (ip ? ipv6) "${ip.ipv6.prefix}::/${toString ip.ipv6.prefixLength}"
    ++ my-lib.netbird.addresses host;
  sending = [
    "authenticate"
    "emailSend"
  ];
in
{
  bundles.stalwart.programAccounts = {
    forgejo = {
      permissions = sending;
      allowedIps = from "oracle-a1-flex-1";
      description = "Forgejo on oracle-a1-flex-1";
    };
    zitadel = {
      permissions = sending;
      allowedIps = from "hetzner-cx23-2";
      description = "ZITADEL on hetzner-cx23-2";
    };
  };
}
