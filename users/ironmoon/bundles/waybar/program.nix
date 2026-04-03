{ config, lib, ... }:
let
  base00 = "#181818"; # #181818
  base01 = "#2b2e37"; # #2b2e37
  base02 = "#3b3e47"; # #3b3e47
  base03 = "#585858"; # #585858
  base04 = "#b8b8b8"; # #b8b8b8
  base05 = "#d8d8d8"; # #d8d8d8
  base06 = "#e8e8e8"; # #e8e8e8
  base07 = "#f8f8f8"; # #f8f8f8
  base08 = "#ab4642"; # #ab4642
  base09 = "#dc9656"; # #dc9656
  base0A = "#f7ca88"; # #f7ca88
  base0B = "#a1b56c"; # #a1b56c
  base0C = "#86c1b9"; # #86c1b9
  base0D = "#7cafc2"; # #7cafc2
  base0E = "#ba8baf"; # #ba8baf
  base0F = "#a16946"; # #a16946
in
{
  enable = config.host.hyprland;

  systemd = {
    enable = true;
    targets = ["wayland-session@hyprland.desktop.target"];
  };

  settings = {
    mainBar = {
      "layer" = "top";
      "position" = "top";

      "spacing" = 2;

      "height" = 30;

      "modules-left" = [
        "hyprland/workspaces"
        "hyprland/window"
      ];

      "modules-center" = [
        "clock"
      ];

      "modules-right" = [
        "tray"

        "cpu"
        "memory"
        "temperature"
        "backlight"

        # "network"
        "pulseaudio"

        "power-profiles-daemon"
        "idle_inhibitor"
        "battery"
        # "custom/dunst"
      ];

      "hyprland/workspaces" = {
        "format" = "{icon}";
        "on-scroll-up" = "hyprctl dispatch workspace e+1";
        "on-scroll-down" = "hyprctl dispatch workspace e-1";
      };

      "hyprland/window" = {
        "separate-outputs" = true;
        "max-length" = 150;
        "icon" = true;
        "icon-size" = 16;
      };

      # -----------------

      "clock" = {
        "interval" = 1;
        "format" = "{:%a %Y-%m-%d %H:%M:%S}";
        "on-click" = "";
        "on-click-middle" = "";
        "on-click-right" = "";
        "tooltip-format" = "<big>{:%Y %B}</big>\n<tt>{calendar}</tt>";
      };

      # -----------------

      "tray" = {
        "spacing" = 0;
      };

      "cpu" = {
        "interval" = 1;
        "format" =
          if config.host.cpu-cores != null then
            config.host.cpu-cores |> builtins.genList (i: "{icon${builtins.toString i}}") |> lib.concatStrings
          else
            "{usage}% ";
        "format-icons" = [
          " "
          "<span color='#69ff94'>▁</span>" # #69ff94
          "<span color='#2aa9ff'>▂</span>" # #2aa9ff
          "<span color='#f8f8f2'>▃</span>" # #f8f8f2
          "<span color='#f8f8f2'>▄</span>" # #f8f8f2
          "<span color='#ffffa5'>▅</span>" # #ffffa5
          "<span color='#ffffa5'>▆</span>" # #ffffa5
          "<span color='#ff9977'>▇</span>" # #ff9977
          "<span color='#dd532e'>█</span>" # #dd532e
        ];
      };
      "memory" = {
        "format" = "{}% ";
      };
      "temperature" = {
        "critical-threshold" = 80;
        "format" = "{temperatureC}°C {icon}";
        "format-icons" = [
          ""
          ""
          ""
        ];
      };
      "backlight" = {
        "format" = "{percent}% {icon}";
        "format-icons" = [
          ""
          ""
          ""
          ""
          ""
          ""
          ""
          ""
          ""
        ];
      };

      "network" = {
        "format-wifi" = "{essid} ({signalStrength}%) ";
        "format-ethernet" = "{ipaddr}/{cidr} 󰈀";
        "tooltip-format" = "{ifname} via {gwaddr} 󰈀";
        "format-linked" = "{ifname} (No IP) 󰈀";
        "format-disconnected" = "Disconnected ⚠";
        "format-alt" = "{ifname}: {ipaddr}/{cidr}";
      };

      "pulseaudio" = {
        "scroll-step" = 5;
        "format" = "{volume}% {icon} {format_source}";
        "format-bluetooth" = "{volume}% {icon} {format_source}";
        "format-bluetooth-muted" = "󰝟 {icon} {format_source}";
        "format-muted" = "<span color='${base0F}'>󰝟</span> {format_source}";
        "format-source" = "{volume}% <span color='${base08}'></span>";
        "format-source-muted" = " ";
        "format-icons" = {
          "headphone" = "";
          "hands-free" = "󰋎";
          "headset" = "󰋎";
          "phone" = "";
          "portable" = "";
          "car" = "";
          "default" = [
            ""
            ""
            ""
          ];
        };
        "on-click" = "pwvucontrol";
      };

      "power-profiles-daemon" = {
        "format" = "{icon}";
        "tooltip-format" = "Power profile: {profile}\nDriver: {driver}";
        "tooltip" = true;
        "format-icons" = {
          # "default" = "";
          "performance" = "";
          # these are rendered too large
          "balanced" = " ";
          "power-saver" = " ";
        };
      };
      "idle_inhibitor" = {
        "format" = "{icon}";
        "format-icons" = {
          "activated" = "";
          "deactivated" = "";
        };
      };
      "battery" = {
        "interval" = 15;
        "states" = {
          "warning" = 30;
          "critical" = 15;
        };
        "format" = "{capacity}% {icon}";
        "format-warning" = "{capacity}% <span color='${base09}'>{icon}</span>";
        "format-critical" = "{capacity}% <span color='${base08}'>{icon}</span>";
        "format-plugged" = "{capacity}% ";
        "tooltip" = true;
        "tooltip-format" = "{timeTo}\nCycles: {cycles}\nHealth: {health}%\nPower: {power}W";
        "format-icons" = {
          charging = [
            "󰢟"
            "󰢜"
            "󰂆"
            "󰂇"
            "󰂈"
            "󰢝"
            "󰂉"
            "󰢞"
            "󰂊"
            "󰂋"
            "󰂅"
          ];
          default = [
            "󰂎"
            "󰁺"
            "󰁻"
            "󰁼"
            "󰁽"
            "󰁿"
            "󰂀"
            "󰂁"
            "󰂂"
            "󰁹"
          ];
        };
      };
      # "custom/dunst" = {
      #   "on-click" = "dunstctl set-paused toggle";
      # };
    };
  };

  style = builtins.readFile ./waybar.css;
}
