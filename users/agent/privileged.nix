# What a privileged account has beyond the others: it files requests at the approval socket, whose group holds him and
# the privileged accounts alone, and it runs as any account that isn't privileged, to move their lanes. Nothing runs as
# it but him.
{ lib, ... }:
let
  accounts = builtins.partition (a: a.privileged) (builtins.attrValues (import ./accounts.nix));
  names = map (a: a.name);
in
{
  users.groups.approve-requesters.members = [ "ironmoon" ] ++ names accounts.right;
  agent.groupsWithHim = [ "approve-requesters" ];
  security.sudo.extraRules = lib.optional (accounts.right != [ ] && accounts.wrong != [ ]) {
    users = names accounts.right;
    runAs = lib.concatStringsSep "," (names accounts.wrong);
    commands = [
      {
        command = "ALL";
        options = [ "NOPASSWD" ];
      }
    ];
  };
}
