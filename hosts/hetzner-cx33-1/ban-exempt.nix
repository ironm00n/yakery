{ lib, my-lib, ... }:
{
  bundles.stalwart.banExempt = {
    "Northeastern University (AS156)" = [
      "129.10.0.0/16"
      "144.91.0.0/18"
      "155.33.0.0/16"
      "204.167.52.0/24"
    ];
    # all their sessions sign in as agent@ and reconnect together after a restart
    "desktop, fw12 and fw13 over NetBird" = lib.concatMap my-lib.netbird.addresses [
      "desktop"
      "fw12"
      "fw13"
    ];
  };
}
