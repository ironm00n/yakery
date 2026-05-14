{
  pkgs,
  lib,
  config,
  ...
}:
let
  inherit (pkgs) pm2;
  user = "tanzanite";
  userHome = "/var/lib/${user}";
in
{
  environment.systemPackages = [
    pm2
  ];

  users.users.${user} = {
    isSystemUser = true;
    group = user;
    shell = pkgs.bashInteractive;
    home = userHome;
    createHome = true;
    openssh.authorizedKeys.keys = config.users.users.root.openssh.authorizedKeys.keys;
    packages = with pkgs; [
      nodejs
      yarn
      git
    ];
  };
  users.groups.${user} = { };

  systemd.services.pm2 = {
    description = "pm2";
    wantedBy = [ "multi-user.target" ];
    after = [ "network.target" ];

    environment = {
      PM2_HOME = "${userHome}/.pm2";
      HOME = userHome;
      PATH = lib.mkForce "/etc/profiles/per-user/${user}/bin:/run/current-system/sw/bin";
    };

    serviceConfig = {
      Type = "forking";
      User = user;
      Group = user;
      PIDFile = "${userHome}/.pm2/pm2.pid";
      ExecStart = "${lib.getExe pm2} resurrect";
      ExecReload = "${lib.getExe pm2} reload all";
      ExecStop = "${lib.getExe pm2} kill";
      Restart = "on-failure";
    };
  };
}
