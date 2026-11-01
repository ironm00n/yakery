(import-macros {: bind : binds} :lib.macros)
(import-macros hl! :lib.checked)

(local nix (require! :nix))

;; long-lived apps get their own app-*.scope instead of pooling into the
;; compositor's cgroup, where oomd can only kill them by killing the session
(fn app [cmd] (.. "uwsm app -- " cmd))

(local menu :anyrun)

(fn vm-clip []
  (binds {}
    "SUPER + SHIFT + V" nix.clip-to-vm
    "SUPER + CTRL + V" nix.clip-from-vm))

;; https://wiki.hyprland.org/Configuring/Binds/, wev to inspect
(binds {}
  "SUPER + T" (app :kitty)
  "SUPER + E" (app "doom emacs")
  "SUPER + D" (app :dolphin)
  "SUPER + B" (app "firefox -new-tab")
  "SUPER + Q" (hl.dsp.window.close)
  "SUPER + SHIFT + ALT + Q" (hl.dsp.window.kill)
  "SUPER + SHIFT + Delete" "uwsm stop"
  "SUPER + F" (hl.dsp.window.float {:action :toggle})
  "SUPER + G" (hl.dsp.window.fullscreen)
  "SUPER + SPACE" menu
  "ALT + SPACE" menu
  "SUPER + P" (hl.dsp.window.pseudo) ; dwindle
  "SUPER + J" (hl.dsp.layout :togglesplit) ; dwindle

  ;; clipboard
  "SUPER + V" "cliphist list | wofi --dmenu --cache-file=/dev/null | cliphist decode | wl-copy"

  ;; screenshot
  "SUPER + PRINT" "hyprshot -m window"
  "PRINT" "hyprshot -m output"
  "SUPER + SHIFT + PRINT" "hyprshot -m region -z"
  "SUPER + SHIFT + s" "hyprshot -m region -z"

  ;; color picker
  "SUPER + SHIFT + c" "hyprpicker -a -n -q"

  ;; lock
  "SUPER + L" "loginctl lock-session"

  ;; Example special workspace (scratchpad)
  "SUPER + grave" (hl.dsp.workspace.toggle_special :magic)
  "SUPER + SHIFT + grave" (hl.dsp.window.move {:workspace "special:magic"})

  ;; Scroll through existing workspaces with mainMod + scroll
  "SUPER + mouse_down" (hl.dsp.focus {:workspace :e+1})
  "SUPER + mouse_up" (hl.dsp.focus {:workspace :e-1})

  "SUPER + CTRL + left" (hl.dsp.focus {:workspace :m-1})
  "SUPER + CTRL + right" (hl.dsp.focus {:workspace :m+1})
  "SUPER + CTRL + ALT + left" (hl.dsp.focus {:workspace :r-1})
  "SUPER + CTRL + ALT + right" (hl.dsp.focus {:workspace :r+1}))

(vm-clip)

;; hand every other bind to the focused window (VMs, nested compositors)
(bind "SUPER + Escape" (hl.dsp.submap :passthrough))

;; a submap disables every bind outside it, so this one is the whole layer
(hl.define_submap :passthrough
  (fn []
    (bind "SUPER + Escape" (hl.dsp.submap :reset))
    (vm-clip)))

;; Move focus with mainMod + arrow keys
(each [_ dir (ipairs [:left :right :up :down])]
  (bind (.. "SUPER + " dir) (hl.dsp.focus {:direction dir}))
  (bind (.. "SUPER + SHIFT + " dir) (hl.dsp.window.move {:direction dir})))

;; zooming, inspired by https://reddit.com/comments/1c61h25/-/m444x7r
(fn zoom [f]
  #(hl!.config {:cursor {:zoom_factor (math.max 1 (f (hl.get_config :cursor.zoom_factor)))}}))

(binds {:repeating true}
  "SUPER + equal" (zoom #(* $ 1.1))
  "SUPER + minus" (zoom #(* $ 0.9))
  "SUPER + BackSpace" (zoom #1))

;; Move/resize windows with mainMod + LMB/RMB and dragging
(binds {:mouse true}
  "SUPER + mouse:272" (hl.dsp.window.drag)
  "SUPER + mouse:273" (hl.dsp.window.resize))

;; Laptop multimedia keys for volume and LCD brightness
(binds {:locked true :repeating true}
  :XF86AudioRaiseVolume "wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%+"
  :XF86AudioLowerVolume "wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%-"
  :XF86AudioMute "wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle"
  :XF86AudioMicMute "wpctl set-mute @DEFAULT_AUDIO_SOURCE@ toggle"
  :XF86MonBrightnessUp "brightnessctl s 5%+"
  :XF86MonBrightnessDown "brightnessctl s 5%-")

;; Requires playerctl
(binds {:locked true}
  :XF86AudioNext "playerctl next"
  :XF86AudioPause "playerctl play-pause"
  :XF86AudioPlay "playerctl play-pause"
  :XF86AudioPrev "playerctl previous")
