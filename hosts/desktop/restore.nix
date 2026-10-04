{
  config,
  inputs,
  lib,
  my-lib,
  pkgs,
  ...
}:
let
  borgKey = my-lib.sops.mkSecrets {
    inherit config;
    sopsFile = inputs.secrets.lib.borg.desktop;
    prefix = "borg-desktop";
  } [ "passphrase" ];

  hopRsh = pkgs.writeShellScript "borg-hop-rsh" ''
    exec ${lib.getExe pkgs.socat} STDIO TCP:hetzner-cx23-1.h.im.exposed:8023
  '';

  userState = [
    "home/ironmoon"
    "root"
    "etc/nixos"
    "etc/secrets"
    "etc/NetworkManager/system-connections"
    "etc/mullvad-vpn"
    "opt/factorio"
  ]
  ++ map (d: "var/lib/${d}") [
    "libvirt"
    "waydroid"
    "minecraft"
    "bluetooth"
    "AccountsService"
  ];

  # --numeric-ids: the reinstall allocates the host's system uids afresh
  containerStores = [
    "var/lib/docker"
    "var/lib/containers"
    "var/lib/lxc"
    "var/lib/machines"
    "home/ironmoon/.local/share/containers"
  ];

  quiesce = [
    "docker.socket"
    "docker.service"
    "containerd.service"
    "libvirtd.socket"
    "libvirtd.service"
    "waydroid-container.service"
    "bluetooth.service"
    "mullvad-daemon.service"
    "accounts-daemon.service"
    "user@1000.service"
  ];
in
{
  sops.secrets = borgKey.secrets;

  systemd.services.desktop-restore = {
    description = "Restore user state from the newest pre-luks borg archive";
    wants = [ "network-online.target" ];
    after = [ "network-online.target" ];
    path = [
      pkgs.borgbackup
      pkgs.gawk
      pkgs.networkmanager
      pkgs.shadow
    ];
    environment = {
      BORG_RSH = "${hopRsh}";
      BORG_PASSCOMMAND = "cat ${borgKey.get-path "passphrase"}";
      BORG_BASE_DIR = "/var/lib/desktop-restore";
      BORG_RELOCATED_REPO_ACCESS_IS_OK = "yes";
    };
    serviceConfig = {
      Type = "oneshot";
      StateDirectory = "desktop-restore";
      WorkingDirectory = "/";
      TimeoutStartSec = "infinity";
    };
    script = ''
      repo=ssh://hop/./desktop
      log=/var/lib/desktop-restore/warnings.log
      : > "$log"
      newest() { borg list --short --glob-archives "$1" --last 1 "$repo"; }
      archive=$(newest 'pre-luks-*')
      containers=$(newest 'containers-*')
      echo "restoring from $archive and ''${containers:-no containers-* archive}"
      systemctl stop ${toString quiesce} || true
      rc=0 crc=0
      borg extract --sparse --exclude home/ironmoon/.local/share/containers \
        "$repo::$archive" ${toString userState} 2> >(tee -a "$log" >&2) || rc=$?
      if [ -n "$containers" ]; then
        borg extract --sparse --numeric-ids \
          "$repo::$containers" ${toString containerStores} 2> >(tee -a "$log" >&2) || crc=$?
      else
        echo "WARNING: no containers-* archive; container stores were not restored" | tee -a "$log"
        crc=2
      fi
      nmcli connection reload || true
      hash=$(borg extract --stdout "$repo::$archive" etc/shadow | awk -F: '$1 == "ironmoon" { print $2 }')
      if [ -n "$hash" ] && printf 'ironmoon:%s\n' "$hash" | chpasswd -e; then
        echo "restored the ironmoon password hash"
      else
        echo "WARNING: the ironmoon password hash was not restored; set one with passwd" | tee -a "$log"
      fi
      echo "borg extract rc $rc, containers rc $crc; $(wc -l < "$log") warning lines in $log"
      [ "$rc" -le 1 ] && [ "$crc" -le 1 ]
    '';
  };
}
