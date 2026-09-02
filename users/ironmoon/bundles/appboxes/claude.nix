{ config, pkgs, ... }:
{
  bundles.appboxes.boxes.claude = {
    hostPaths = [ "/etc/nixos" ];
    # the app's state stays in the box home; Claude Code's config, memory and credentials are the real ones
    environment.CLAUDE_CONFIG_DIR = "${config.home.homeDirectory}/.claude";
    aptRepos.claude-desktop = {
      uri = "https://downloads.claude.ai/claude-desktop/apt/stable";
      suite = "stable";
      architectures = [
        "amd64"
        "arm64"
      ];
      key = pkgs.fetchurl {
        url = "https://downloads.claude.ai/claude-desktop/key.asc";
        sha256 = "1il1ps9fba933k8vvw7kji7129308kwagssc0822f038lbjaaw5x";
      };
      fingerprint = "31DDDE24DDFAB679F42D7BD2BAA929FF1A7ECACE";
    };
    apps.claude = {
      name = "Claude";
      exec = "claude-desktop --password-store=gnome-libsecret %U";
      icon = "/usr/share/icons/hicolor/256x256/apps/claude-desktop.png";
      urlSchemes = [ "claude" ];
      categories = [ "Development" ];
    };
    apps.claude-science = {
      name = "Claude Science";
      # its sandbox socket lives under the data dir and ~/.claude-science overruns AF_UNIX's 108 bytes
      exec = "claude-science serve --data-dir ${config.bundles.appboxes.boxes.claude.home}/.cs --detached";
      categories = [ "Science" ];
    };
  };
}
