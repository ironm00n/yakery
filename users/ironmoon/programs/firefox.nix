{ ... }:
let
  common-settings = {
    "widget.use-xdg-desktop-portal.file-picker" = 1;
    "browser.tabs.groups.enabled" = true;
    "ui.key.menuAccessKeyFocuses" = false;
    "font.name-list.emoji" = "Twemoji COLR";
  };
in
{
  enable = true;

  # see policies in hosts/common/programs/firefox.nix

  profiles = {
    default = {
      id = 0;
      settings = common-settings;
    };

    dev-edition-default = {
      id = 1;
      settings = common-settings;
    };
  };
}

# useful shortcuts (https://support.mozilla.org/en-US/kb/keyboard-shortcuts-perform-firefox-tasks-quickly):
# - `ctrl+[` back
# - `ctrl+]` forwards
# - `ctrl+shift+r` reload (without cache)
# - `alt+d` focus address bar (keeping current content)
# - `alt+f` open file menu
# - `alt+g` find again
