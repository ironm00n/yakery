# see https://sw.kovidgoyal.net/kitty/conf/
{ pkgs, ... }:
{
  enable = true;

  # font.name = "FiraCode Nerd Font";
  font.name = "JetBrainsMono Nerd Font"; # FiraCode doesn't have italic
  settings = {
    update_check_interval = 0;
    scrollback_lines = 20000;
    enable_audio_bell = false;
    # scrollback_pager = "moar --no-linenumbers"; # todo: INPUT_LINE_NUMBER
    bold_font = "JetBrainsMono Nerd Font:style=ExtraBold";
  };
}
