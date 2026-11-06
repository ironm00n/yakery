{ inputs, my-lib, ... }:
let
  # NetBird's DNS sends a peer to this host's overlay addresses, so it signs in from its own
  from = host: [ inputs.secrets.data.ips.${host}.ipv4.address ] ++ my-lib.netbird.addresses host;
in
{
  bundles.stalwart.programAccounts.forgejo = {
    permissions = [
      "authenticate"
      "emailSend"
    ];
    allowedIps = from "oracle-a1-flex-1";
    description = "Forgejo on oracle-a1-flex-1";
  };
}
