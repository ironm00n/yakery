{ config, pkgs, ... }:
{
  enable = config.host.hyprland;
  config = {
    x = {
      fraction = 0.5;
    };
    y = {
      absolute = 10;
    };
    width = {
      fraction = 0.3;
    };
    height = {
      absolute = 0;
    };
    hideIcons = false;
    ignoreExclusiveZones = false;
    layer = "top";
    hidePluginInfo = false;
    closeOnClick = true;
    showResultsImmediately = false;
    maxEntries = null;

    plugins = with pkgs; [
      "${anyrun}/lib/libapplications.so"
      "${anyrun}/lib/librink.so"
      "${anyrun}/lib/libsymbols.so"
      "${anyrun}/lib/libdictionary.so"
    ];
  };

  # FIXME: why isnt default loading?
  extraCss = /* css */ ''
    window {
      background: transparent;
    }

    box.main {
      padding: 2px;
      margin: 10px;
      border-radius: 5px;
      background-color: @theme_bg_color;
      box-shadow: 0 0 5px black;
    }

    text {
      min-height: 30px;
      padding: 5px;
      border-radius: 5px;
    }

    .matches {
      background-color: rgba(0, 0, 0, 0);
      border-radius: 5px;
    }

    box.plugin:first-child {
      margin-top: 5px;
    }

    box.plugin.info {
      min-width: 200px;
    }

    list.plugin {
      background-color: rgba(0, 0, 0, 0);
    }

    label.match.description {
      font-size: 10px;
    }

    label.plugin.info {
      font-size: 14px;
    }

    .match {
      background: transparent;
    }

    .match:selected {
      background: @theme_selected_bg_color;
      animation: fade 0.1s linear;
    }

    @keyframes fade {
      0% {
        opacity: 0;
      }

      100% {
        opacity: 1;
      }
    }
  '';

  extraConfigFiles = {
    "dictionary.ron".text = /* ron */ ''
      Config(
        prefix: ":def",
        max_entries: 10,
      )
    '';
    "symbols.ron".text = /* ron */ ''
      Config(
        prefix: ":sym",
        max_entries: 10,
        symbols: {
          "shrug": "¯\\_(ツ)_/¯",
        },
      )
    '';
  };
}
