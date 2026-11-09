# The groups no agent account may share with him, checked at evaluation: each case is a host and the accounts it must
# refuse, by group.
{ pkgs }:
let
  inherit (pkgs) lib;
  accounts = builtins.partition (a: a.privileged) (
    builtins.attrValues (import ../../users/agent/accounts.nix)
  );
  privileged = (builtins.head accounts.right).name;
  plain = (builtins.head accounts.wrong).name;
  refused =
    host:
    let
      inherit
        (pkgs.nixos {
          imports = [
            ../../users/agent/user.nix
            ../../users/agent/shares.nix
            ../../users/agent/privileged.nix
            host
          ];
          boot.isContainer = true;
          system.stateVersion = lib.trivial.release;
          users.users.ironmoon.isNormalUser = true;
        })
        config
        ;
    in
    map (a: builtins.head (lib.splitString ", which" a.message)) (
      lib.filter (
        a: !a.assertion && lib.hasInfix "which no agent account may join" a.message
      ) config.assertions
    );
  cases = {
    shared-on-purpose = {
      host.users.users.ironmoon.extraGroups = [
        "wheel"
        "input"
      ];
      expected = [ ];
    };
    his-keyboard = {
      host = {
        users.users.ironmoon.extraGroups = [ "input" ];
        users.users.${plain}.extraGroups = [ "input" ];
      };
      expected = [ "users/agent: ${plain} is in input" ];
    };
    his-primary-group = {
      host.users.groups.users.members = [ privileged ];
      expected = [ "users/agent: ${privileged} is in users" ];
    };
    wheel-once-he-leaves-it = {
      host.users.users.${privileged}.extraGroups = [
        "wheel"
        "docker"
      ];
      expected = [ "users/agent: ${privileged} is in wheel, docker" ];
    };
  };
  wrong = lib.filterAttrs (_: case: refused case.host != case.expected) cases;
in
pkgs.runCommand "agent-groups" { } (
  if wrong == { } then
    "touch $out"
  else
    throw "agent-groups: ${builtins.toJSON (lib.mapAttrs (_: case: refused case.host) wrong)}"
)
