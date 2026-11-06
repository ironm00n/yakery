rec {
  overlay = {
    v4 = "100.69.0.0/16";
    v6 = "fd69::/64";
  };
  overlayCidrs = [
    overlay.v4
    overlay.v6
  ];

  # Overlay addresses are handed out by NetBird management; a peer is listed once config needs its address.
  peers = {
    desktop = {
      v4 = "100.69.0.1";
      v6 = "fd69::1";
    };
    hetzner-cx33-1 = {
      v4 = "100.69.57.252";
      v6 = "fd69::d9cb:7dd5:5e8a:7b1d";
    };
    oracle-a1-flex-1 = {
      v4 = "100.69.2.1";
      v6 = "fd69::2:1";
    };
  };
  addresses = host: builtins.attrValues (peers.${host} or { });
}
