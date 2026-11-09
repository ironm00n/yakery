# xb for each account: its client is whatever xb's own deploys last put in /var/lib/xb, so updating xb needs no
# switch. The client finds xb's keeper itself, which admits the account by its uid, read from /etc/xb/agent-uids.
{ lib, pkgs, ... }:
let
  current = "/var/lib/xb/current";
  accounts = import ./accounts.nix;
in
{
  systemd.tmpfiles.rules = [ "d /var/lib/xb 0755 ironmoon - -" ];
  environment.etc."xb/agent-uids".text = lib.concatLines (
    [ "# from users/agent/accounts.nix" ] ++ lib.mapAttrsToList (_: a: toString a.uid) accounts
  );
  home-manager.users = builtins.mapAttrs (
    _: _:
    { config, ... }:
    {
      home.packages = [
        (pkgs.runCommand "xb" { } "mkdir -p $out/bin && ln -s ${current}/bin/xb $out/bin/xb")
      ];
      # jj-push.sh runs the pre-push gate with the xb found here
      home.file.".local/state/xb/current".source = config.lib.file.mkOutOfStoreSymlink current;
    }
  ) accounts;
}
