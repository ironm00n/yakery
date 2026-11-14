# Its xb client is whatever the privileged account last placed (../xb.nix), so updating xb needs no deploy.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (import ../layout.nix { inherit lib; }) xb;
in
{
  home.packages = [
    (pkgs.runCommand "xb" { } "mkdir -p $out/bin && ln -s ${xb}/bin/xb $out/bin/xb")
  ];
  # jj-push.sh runs the pre-push gate with the xb found here
  home.file.".local/state/xb/current".source = config.lib.file.mkOutOfStoreSymlink xb;
}
