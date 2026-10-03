{ config, inputs, ... }:
let
  inherit (inputs.secrets.data.ips.${config.host.hostname}) ipv4 ipv6;
in
{
  networking = {
    interfaces.enp1s0 = {
      ipv6.addresses = [
        {
          address = "${ipv6.prefix}::1";
          inherit (ipv6) prefixLength;
        }
      ];
      ipv4.addresses = [
        {
          inherit (ipv4) address prefixLength;
        }
      ];
    };
    defaultGateway6 = {
      address = ipv6.gateway;
      interface = "enp1s0";
    };
    defaultGateway = {
      address = ipv4.gateway;
      interface = "enp1s0";
    };
    nameservers = [
      "2606:4700:4700::1111"
      "2606:4700:4700::1001"
      "1.1.1.1"
      "1.0.0.1"
    ];
  };
}
