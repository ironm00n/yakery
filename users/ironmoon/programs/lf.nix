{ pkgs, lib, ... }:
let
  inherit (lib) getExe getExe';

  icat = "${getExe' pkgs.kitty "kitten"} icat --stdin no --transfer-mode file";

  # lf renders the previewer's stdout inside the pane, so graphics escapes have to
  # bypass it; the non-zero exit drops the preview from lf's cache, which is what
  # makes lf invoke the cleaner and redraw the image on every revisit.
  previewer = pkgs.writeShellApplication {
    name = "lf-preview";
    runtimeInputs = with pkgs; [
      file
      bat
    ];
    text = ''
      path=$1 width=$2 height=$3 left=$4 top=$5 mode=$6

      case "$(file -Lb --mime-type -- "$path")" in
      image/*)
        if [ "$mode" = preview ]; then
          ${icat} --place "''${width}x''${height}@''${left}x''${top}" "$path" >/dev/tty
        fi
        exit 1
        ;;
      esac

      bat --color=always --style=plain --paging=never -- "$path"
    '';
  };

  cleaner = pkgs.writeShellApplication {
    name = "lf-clean";
    text = ''
      ${icat} --clear >/dev/tty
    '';
  };
in
{
  enable = true;
  settings.cleaner = getExe cleaner;
  previewer.source = getExe previewer;
}
