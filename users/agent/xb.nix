# xb for each account: its client is whatever the privileged account last put in /var/lib/xb (its home links it,
# home/xb.nix), so updating xb needs no switch. The client finds xb's keeper itself, which admits the account by its
# uid, read from /etc/xb/agent-uids.
{ lib, ... }:
let
  accounts = import ./accounts.nix;
  placer =
    lib.findSingle (a: a.privileged) (throw "users/agent/xb.nix: no privileged account places xb")
      (throw "users/agent/xb.nix: more than one privileged account would place xb")
      (builtins.attrValues accounts);
in
{
  systemd.tmpfiles.rules = [
    "d ${dirOf (import ./layout.nix { inherit lib; }).xb} 0755 ${placer.name} - -"
  ];
  environment.etc."xb/agent-uids".text = lib.concatLines (
    [ "# from users/agent/accounts.nix" ] ++ lib.mapAttrsToList (_: a: toString a.uid) accounts
  );
}
