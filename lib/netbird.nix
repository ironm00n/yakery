rec {
  overlay = {
    v4 = "100.69.0.0/16";
    v6 = "fd69::/64";
  };
  overlayCidrs = [
    overlay.v4
    overlay.v6
  ];
}
