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
  # Forgejo's incoming email: it waits in IDLE, fetches what is unseen, and flags and expunges what it handled
  reading = [
    "emailReceive"
    "imapAuthenticate"
    "imapCapability"
    "imapSelect"
    "imapSearch"
    "imapFetch"
    "imapStore"
    "imapExpunge"
    "imapIdle"
  ];
  inherit (my-lib.ironmoon.mail) senders;
in
{
  bundles.stalwart.programAccounts = {
    forgejo = {
      permissions = sending ++ reading;
      allowedIps = from "oracle-a1-flex-1";
      aliases = [ senders.forgejo ];
      receivesSubaddressed = [ senders.forgejo ];
      description = "Forgejo on oracle-a1-flex-1";
    };
    zitadel = {
      permissions = sending;
      allowedIps = from "hetzner-cx23-2";
      aliases = [ senders.zitadel ];
      description = "ZITADEL on hetzner-cx23-2";
    };
  };
}
