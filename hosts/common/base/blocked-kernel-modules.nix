{ lib, pkgs, ... }:
let
  # nginx2root gets root through an unpatched use-after-free in kernel SCTP, which nothing here uses
  blocked = [
    "sctp"
    "sctp_diag"
  ];
in
{
  boot.blacklistedKernelModules = blocked;
  boot.extraModprobeConfig = lib.concatMapStrings (
    module: "install ${module} ${lib.getExe' pkgs.coreutils "false"}\n"
  ) blocked;
}
